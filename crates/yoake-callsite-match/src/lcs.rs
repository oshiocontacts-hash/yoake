use crate::{Record, Site};

/// A 1-to-1 aligned `(old_idx, new_idx)` statement pair from LCS diffing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AlignedStmt {
    pub old_idx: usize,
    pub new_idx: usize,
    pub identical: bool,
}

/// Computes 1-to-1 aligned statement pairs using unique LCS anchors
/// plus 1-to-1 replacement gaps bounded by those anchors.
pub fn align_statements(old_stmts: &[u64], new_stmts: &[u64]) -> Vec<AlignedStmt> {
    let m = old_stmts.len();
    let n = new_stmts.len();
    if m == 0 || n == 0 {
        return Vec::new();
    }

    let stmts_equal = |i: usize, j: usize| old_stmts[i] != 0 && old_stmts[i] == new_stmts[j];

    let mut pref = vec![vec![0usize; n + 1]; m + 1];
    for i in 0..m {
        for j in 0..n {
            pref[i + 1][j + 1] = if stmts_equal(i, j) {
                pref[i][j] + 1
            } else {
                pref[i + 1][j].max(pref[i][j + 1])
            };
        }
    }

    let mut suff = vec![vec![0usize; n + 1]; m + 1];
    for i in (0..m).rev() {
        for j in (0..n).rev() {
            suff[i][j] = if stmts_equal(i, j) {
                suff[i + 1][j + 1] + 1
            } else {
                suff[i + 1][j].max(suff[i][j + 1])
            };
        }
    }

    let total = pref[m][n];
    if total == 0 {
        return Vec::new();
    }

    let mut old_partners: Vec<Vec<usize>> = vec![Vec::new(); m];
    let mut new_partners: Vec<Vec<usize>> = vec![Vec::new(); n];
    for i in 0..m {
        for j in 0..n {
            if stmts_equal(i, j) && pref[i][j] + 1 + suff[i + 1][j + 1] == total {
                old_partners[i].push(j);
                new_partners[j].push(i);
            }
        }
    }

    let mut anchors: Vec<(usize, usize)> = Vec::new();
    for i in 0..m {
        if old_partners[i].len() == 1 {
            let j = old_partners[i][0];
            if new_partners[j].len() == 1 && anchors.last().is_none_or(|&(_, pj)| pj < j) {
                anchors.push((i, j));
            }
        }
    }

    if anchors.is_empty() {
        return Vec::new();
    }

    let mut aligned = Vec::new();
    let mut prev_old = 0usize;
    let mut prev_new = 0usize;
    for &(ao, an) in &anchors {
        if ao - prev_old == 1 && an - prev_new == 1 {
            aligned.push(AlignedStmt {
                old_idx: prev_old,
                new_idx: prev_new,
                identical: false,
            });
        }
        aligned.push(AlignedStmt {
            old_idx: ao,
            new_idx: an,
            identical: true,
        });
        prev_old = ao + 1;
        prev_new = an + 1;
    }
    if m - prev_old == 1 && n - prev_new == 1 {
        aligned.push(AlignedStmt {
            old_idx: prev_old,
            new_idx: prev_new,
            identical: false,
        });
    }
    aligned
}

/// Maps records to their unique top-level statement index in `body_stmts`, if unambiguous.
pub fn map_records_to_stmts(records: &[&Record], body_stmts: &[u64]) -> Vec<Option<usize>> {
    records
        .iter()
        .map(|rec| {
            let Some(&top_hash) = rec.stmt_path_hashes.first() else {
                return None;
            };
            let is_top_level = rec.stmt_path_hashes.len() == 1;
            let mut matches = (0..body_stmts.len()).filter(|&idx| {
                if body_stmts[idx] != top_hash {
                    return false;
                }
                if !is_top_level {
                    return true;
                }
                let prev = if idx > 0 { body_stmts[idx - 1] } else { 0 };
                let next = if idx + 1 < body_stmts.len() {
                    body_stmts[idx + 1]
                } else {
                    0
                };
                (prev, next) == rec.context
            });
            let first = matches.next()?;
            if matches.next().is_some() {
                None
            } else {
                Some(first)
            }
        })
        .collect()
}

/// Reconstructs a best-effort top-level statement hash sequence from records alone
/// when a `CallerSnapshot` is not supplied.
pub fn reconstruct_old_stmts(records: &[&Record]) -> (Vec<u64>, Vec<Option<usize>>) {
    let mut stmts: Vec<u64> = Vec::new();
    let mut mapped: Vec<Option<usize>> = Vec::with_capacity(records.len());
    let mut pending_next: Option<u64> = None;
    let mut prev_key: Option<(u64, (u64, u64), bool)> = None;

    for rec in records {
        let Some(&top_hash) = rec.stmt_path_hashes.first() else {
            mapped.push(None);
            continue;
        };
        let is_top = rec.stmt_path_hashes.len() == 1;
        let same_stmt = prev_key.is_some_and(|(ph, pctx, ptop)| {
            ph == top_hash && (!is_top || !ptop || pctx == rec.context)
        });
        if same_stmt {
            if let Some(last_idx) = stmts.len().checked_sub(1) {
                mapped.push(Some(last_idx));
                continue;
            }
        }
        if is_top && rec.context.0 != 0 {
            let need_prev = stmts.last().copied() != Some(rec.context.0);
            if need_prev {
                if let Some(pn) = pending_next.take()
                    && pn != rec.context.0
                {
                    stmts.push(pn);
                }
                stmts.push(rec.context.0);
            } else {
                pending_next = None;
            }
        } else if let Some(pn) = pending_next.take()
            && pn != top_hash
        {
            stmts.push(pn);
        }
        let idx = stmts.len();
        stmts.push(top_hash);
        mapped.push(Some(idx));
        pending_next = if is_top && rec.context.1 != 0 {
            Some(rec.context.1)
        } else {
            None
        };
        prev_key = Some((top_hash, rec.context, is_top));
    }
    if let Some(pn) = pending_next {
        stmts.push(pn);
    }
    (stmts, mapped)
}

/// Reconstructs a best-effort top-level statement hash sequence from extracted sites
/// when a `CallerSnapshot` is not supplied.
pub fn reconstruct_new_stmts(sites: &[&Site]) -> (Vec<u64>, Vec<Option<usize>>) {
    let mut max_idx = 0usize;
    let mut has_any = false;
    for site in sites {
        let end = site.stmt_index + usize::from(site.stmt_path.len() == 1 && site.context.1 != 0);
        max_idx = max_idx.max(end);
        has_any = true;
    }
    if !has_any {
        return (Vec::new(), Vec::new());
    }
    let mut stmts = vec![0u64; max_idx + 1];
    for site in sites {
        stmts[site.stmt_index] = site.stmt_hash;
        if site.stmt_path.len() == 1 {
            if site.stmt_index > 0 && site.context.0 != 0 {
                stmts[site.stmt_index - 1] = site.context.0;
            }
            if site.context.1 != 0 {
                stmts[site.stmt_index + 1] = site.context.1;
            }
        }
    }
    let mapped = sites.iter().map(|s| Some(s.stmt_index)).collect();
    (stmts, mapped)
}
