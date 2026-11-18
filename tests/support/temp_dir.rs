pub(crate) fn remove_dir_all(path: impl AsRef<std::path::Path>) {
    let path = path.as_ref();
    for attempt in 0..10 {
        match std::fs::remove_dir_all(path) {
            Ok(()) => return,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && !path.exists() => return,
            Err(error) => {
                if cfg!(windows)
                    && matches!(error.raw_os_error(), Some(5 | 32))
                    && attempt < 9
                {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    continue;
                }
                eprintln!(
                    "Could not clean up temporary directory {}: {error}",
                    path.display()
                );
                return;
            }
        }
    }
}
