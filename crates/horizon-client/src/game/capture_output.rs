use crate::capture_clock::{CaptureFrame, CaptureOutput, CaptureWrite};
use horizon_client_render::gfx::capture::{
    CaptureHandle, CapturePixels, CaptureReadback, CaptureToken,
};
use std::{
    fs::{self, OpenOptions},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError},
    },
    thread::{self, JoinHandle},
};

enum Pixels {
    Gpu(CaptureReadback),
    #[cfg(test)]
    Ready(CapturePixels),
    #[cfg(test)]
    Blocked {
        pixels: CapturePixels,
        started: SyncSender<()>,
        resume: Receiver<()>,
    },
}

impl Pixels {
    fn read(self) -> Result<CapturePixels, String> {
        match self {
            Self::Gpu(readback) => readback.read(),
            #[cfg(test)]
            Self::Ready(pixels) => Ok(pixels),
            #[cfg(test)]
            Self::Blocked {
                pixels,
                started,
                resume,
            } => {
                started.send(()).map_err(|error| error.to_string())?;
                resume.recv().map_err(|error| error.to_string())?;
                Ok(pixels)
            }
        }
    }
}

struct Job {
    frame: CaptureFrame,
    pixels: Pixels,
}

struct Worker {
    sender: SyncSender<Job>,
    complete: Receiver<CaptureWrite>,
    cancelled: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

impl Worker {
    fn start(directory: PathBuf) -> Result<Self, String> {
        let (sender, receiver) = mpsc::sync_channel::<Job>(1);
        let (done, complete) = mpsc::sync_channel(1);
        let cancelled = Arc::new(AtomicBool::new(false));
        let stop = cancelled.clone();
        let thread = thread::Builder::new()
            .name("capture-png".into())
            .spawn(move || {
                while let Ok(job) = receiver.recv() {
                    if stop.load(Ordering::Acquire) {
                        break;
                    }
                    let result = job.pixels.read().and_then(|pixels| {
                        if stop.load(Ordering::Acquire) {
                            return Err("capture output aborted".into());
                        }
                        write_png(&directory.join(job.frame.filename()), pixels)
                    });
                    if stop.load(Ordering::Acquire)
                        || done
                            .send(CaptureWrite {
                                frame: job.frame,
                                result,
                            })
                            .is_err()
                    {
                        break;
                    }
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            sender,
            complete,
            cancelled,
            thread,
        })
    }

    fn join(self, abort: bool) -> Result<(), String> {
        if abort {
            self.cancelled.store(true, Ordering::Release);
        }
        drop(self.sender);
        drop(self.complete);
        self.thread
            .join()
            .map_err(|_| "capture PNG worker panicked".into())
    }
}

fn write_png(path: &Path, pixels: CapturePixels) -> Result<(), String> {
    let [width, height] = pixels.size;
    if width == 0 || height == 0 || pixels.rgba.len() != width as usize * height as usize * 4 {
        return Err("invalid capture RGBA dimensions".into());
    }
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    let mut output = BufWriter::new(file);
    let mut encoder = png::Encoder::new(&mut output, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    let mut png = encoder.write_header().map_err(|error| error.to_string())?;
    png.write_image_data(&pixels.rgba)
        .map_err(|error| error.to_string())?;
    png.finish().map_err(|error| error.to_string())?;
    output.flush().map_err(|error| error.to_string())?;
    output
        .get_ref()
        .sync_all()
        .map_err(|error| error.to_string())
}

pub(super) struct PngCaptureOutput {
    handle: CaptureHandle,
    worker: Option<Worker>,
    outstanding: Option<CaptureFrame>,
    submitted: bool,
    frames: u64,
    next: u64,
    identity: Option<(u64, u64)>,
}

impl PngCaptureOutput {
    pub(super) fn new(handle: CaptureHandle) -> Self {
        Self {
            handle,
            worker: None,
            outstanding: None,
            submitted: false,
            frames: 0,
            next: 1,
            identity: None,
        }
    }

    fn complete(&mut self, result: Result<(), String>) -> Option<CaptureWrite> {
        let frame = self.outstanding.take()?;
        if result.is_ok() {
            self.next += 1;
        }
        Some(CaptureWrite { frame, result })
    }
}

impl CaptureOutput for PngCaptureOutput {
    fn begin(&mut self, directory: &Path, frames: u64) -> Result<(), String> {
        if self.worker.is_some() || frames == 0 || frames == u64::MAX {
            return Err("capture output is active or frame count is invalid".into());
        }
        self.handle.begin()?;
        let worker = fs::create_dir_all(directory)
            .map_err(|error| error.to_string())
            .and_then(|_| Worker::start(directory.to_path_buf()));
        match worker {
            Ok(worker) => self.worker = Some(worker),
            Err(error) => {
                self.handle.abort();
                return Err(error);
            }
        }
        self.frames = frames;
        self.next = 1;
        self.identity = None;
        Ok(())
    }

    fn request_frame(&mut self, frame: CaptureFrame) -> Result<(), String> {
        if self.worker.is_none() || self.outstanding.is_some() {
            return Err("capture output inactive or waiting for the previous PNG".into());
        }
        let identity = (frame.session, frame.generation);
        if frame.time.index != self.next
            || self.next > self.frames
            || self.identity.is_some_and(|expected| expected != identity)
        {
            return Err("capture frame is out of order or belongs to another generation".into());
        }
        self.handle.request(CaptureToken {
            session: frame.session,
            generation: frame.generation,
            index: frame.time.index,
        })?;
        self.identity = Some(identity);
        self.outstanding = Some(frame);
        self.submitted = false;
        Ok(())
    }

    fn poll_complete(&mut self) -> Option<CaptureWrite> {
        let frame = self.outstanding?;
        if let Err(error) = self.handle.check() {
            return self.complete(Err(error));
        }
        if !self.submitted {
            let (token, result) = self.handle.poll()?;
            let expected = CaptureToken {
                session: frame.session,
                generation: frame.generation,
                index: frame.time.index,
            };
            if token != expected {
                return self.complete(Err("capture readback identity mismatch".into()));
            }
            let readback = match result {
                Ok(readback) => readback,
                Err(error) => return self.complete(Err(error)),
            };
            if let Err(error) = self.worker.as_ref()?.sender.try_send(Job {
                frame,
                pixels: Pixels::Gpu(readback),
            }) {
                return self.complete(Err(format!("capture PNG queue: {error}")));
            }
            self.submitted = true;
        }
        match self.worker.as_ref()?.complete.try_recv() {
            Ok(write) if write.frame == frame => self.complete(write.result),
            Ok(_) => self.complete(Err("capture PNG completion identity mismatch".into())),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.complete(Err("capture PNG worker stopped".into()))
            }
        }
    }

    fn finish(&mut self) -> Result<(), String> {
        let checked = self.handle.check();
        let complete = self.outstanding.is_none() && self.next == self.frames + 1;
        let joined = self
            .worker
            .take()
            .map_or(Ok(()), |worker| worker.join(false));
        self.handle.abort();
        checked?;
        joined?;
        if !complete {
            return Err("capture output finished before every PNG completed".into());
        }
        Ok(())
    }

    fn abort(&mut self) {
        self.handle.abort();
        self.outstanding = None;
        self.identity = None;
        if let Some(worker) = self.worker.take() {
            let _ = worker.join(true);
        }
    }
}

impl Drop for PngCaptureOutput {
    fn drop(&mut self) {
        self.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture_clock::FrameTime;
    use std::{
        num::NonZeroU32,
        sync::atomic::AtomicU64,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    struct Directory(PathBuf);

    impl Directory {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
            let path = std::env::temp_dir().join(format!(
                "horizon-capture-{}-{}-{}",
                std::process::id(),
                stamp.as_nanos(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Directory {
        fn drop(&mut self) {
            for entry in fs::read_dir(&self.0).unwrap() {
                fs::remove_file(entry.unwrap().path()).unwrap();
            }
            fs::remove_dir(&self.0).unwrap();
        }
    }

    fn frame(index: u64, generation: u64) -> CaptureFrame {
        CaptureFrame {
            session: 7,
            generation,
            time: FrameTime {
                index,
                fps: NonZeroU32::new(30).unwrap(),
            },
        }
    }

    fn pixels() -> CapturePixels {
        CapturePixels {
            size: [1, 1],
            rgba: vec![12, 34, 56, 255],
        }
    }

    fn receive(worker: &Worker, expected: CaptureFrame) {
        let write = worker
            .complete
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
        assert_eq!(write.frame, expected);
        write.result.unwrap();
    }

    fn submit(worker: &Worker, index: u64, generation: u64) {
        assert!(
            worker
                .sender
                .send(Job {
                    frame: frame(index, generation),
                    pixels: Pixels::Ready(pixels()),
                })
                .is_ok()
        );
    }

    #[test]
    fn worker_writes_numbered_rgba_pngs_in_order() {
        let directory = Directory::new();
        let worker = Worker::start(directory.0.clone()).unwrap();
        for index in 1..=4 {
            submit(&worker, index, 3);
            receive(&worker, frame(index, 3));
            let bytes = fs::read(directory.0.join(frame(index, 3).filename())).unwrap();
            assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
            assert_eq!(&bytes[16..24], &[0, 0, 0, 1, 0, 0, 0, 1]);
            assert_eq!(&bytes[24..26], &[8, 6]);
            assert_eq!(fs::read_dir(&directory.0).unwrap().count(), index as usize);
        }
        worker.join(false).unwrap();
    }

    #[test]
    fn slow_worker_applies_backpressure_without_dropping() {
        let directory = Directory::new();
        let worker = Worker::start(directory.0.clone()).unwrap();
        let (started, waiting) = mpsc::sync_channel(1);
        let (resume, blocked) = mpsc::sync_channel(1);
        assert!(
            worker
                .sender
                .send(Job {
                    frame: frame(1, 3),
                    pixels: Pixels::Blocked {
                        pixels: pixels(),
                        started,
                        resume: blocked,
                    },
                })
                .is_ok()
        );
        waiting.recv_timeout(Duration::from_secs(10)).unwrap();
        submit(&worker, 2, 3);
        let third = match worker.sender.try_send(Job {
            frame: frame(3, 3),
            pixels: Pixels::Ready(pixels()),
        }) {
            Err(mpsc::TrySendError::Full(job)) => job,
            _ => panic!("the bounded queue must stop the producer"),
        };
        assert!(matches!(
            worker.complete.try_recv(),
            Err(TryRecvError::Empty)
        ));
        assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 0);
        resume.send(()).unwrap();
        receive(&worker, frame(1, 3));
        assert!(worker.sender.send(third).is_ok());
        receive(&worker, frame(2, 3));
        receive(&worker, frame(3, 3));
        worker.join(false).unwrap();
        assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 3);
    }

    #[test]
    fn abort_discards_late_old_generation_completion() {
        let directory = Directory::new();
        let worker = Worker::start(directory.0.clone()).unwrap();
        let (started, waiting) = mpsc::sync_channel(1);
        let (resume, blocked) = mpsc::sync_channel(1);
        assert!(
            worker
                .sender
                .send(Job {
                    frame: frame(1, 3),
                    pixels: Pixels::Blocked {
                        pixels: pixels(),
                        started,
                        resume: blocked,
                    },
                })
                .is_ok()
        );
        waiting.recv_timeout(Duration::from_secs(10)).unwrap();
        worker.cancelled.store(true, Ordering::Release);
        resume.send(()).unwrap();
        assert!(
            worker
                .complete
                .recv_timeout(Duration::from_secs(10))
                .is_err()
        );
        worker.join(true).unwrap();
        assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 0);
        let next = Worker::start(directory.0.clone()).unwrap();
        submit(&next, 1, 4);
        receive(&next, frame(1, 4));
        next.join(false).unwrap();
    }
}
