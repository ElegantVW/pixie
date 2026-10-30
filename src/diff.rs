//! In-process line diff for file-touch cards. No extra crate, no `diff` subprocess.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Eq,
    Add,
    Del,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffLine {
    pub op: Op,
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    Read,
    Create,
    Write,
    Search,
    Plan,
    Image,
}

impl FileKind {
    pub fn as_str(self) -> &'static str {
        match self {
            FileKind::Read => "read",
            FileKind::Create => "create",
            FileKind::Write => "write",
            FileKind::Search => "search",
            FileKind::Plan => "plan",
            FileKind::Image => "image",
        }
    }
}

#[derive(Clone, Debug)]
pub struct FileTouch {
    pub path: String,
    pub kind: FileKind,
    pub added: usize,
    pub deleted: usize,
    pub lines: Vec<DiffLine>,
    pub binary: bool,
    pub extra_paths: Vec<String>,
    pub collapsed: bool,
}

impl FileTouch {
    pub fn summary(&self) -> String {
        if self.kind == FileKind::Image {
            "picture".into()
        } else if self.binary {
            "binary".into()
        } else if self.added > 0 || self.deleted > 0 {
            format!("+{} -{}", self.added, self.deleted)
        } else {
            String::new()
        }
    }
}

const DP_CAP: usize = 1500;
const STORE_CAP: usize = 400;

pub fn diff_text(old: &str, new: &str) -> Vec<DiffLine> {
    let old_l: Vec<&str> = old.lines().collect();
    let new_l: Vec<&str> = new.lines().collect();
    cap_store(diff_lines(&old_l, &new_l))
}

pub fn counts(lines: &[DiffLine]) -> (usize, usize) {
    let mut a = 0usize;
    let mut d = 0usize;
    for l in lines {
        match l.op {
            Op::Add => a += 1,
            Op::Del => d += 1,
            Op::Eq => {}
        }
    }
    (a, d)
}

pub fn for_card(lines: &[DiffLine], cap: usize) -> Vec<DiffLine> {
    if lines.len() <= cap {
        return lines.to_vec();
    }
    let mut keep = vec![false; lines.len()];
    for (i, l) in lines.iter().enumerate() {
        if l.op != Op::Eq {
            keep[i] = true;
            if i > 0 {
                keep[i - 1] = true;
            }
            if i + 1 < lines.len() {
                keep[i + 1] = true;
            }
        }
    }
    let mut out = Vec::new();
    let mut skipped = false;
    for (i, l) in lines.iter().enumerate() {
        if keep[i] {
            skipped = false;
            out.push(l.clone());
            if out.len() >= cap {
                break;
            }
        } else if !skipped {
            skipped = true;
            out.push(DiffLine {
                op: Op::Eq,
                text: "· · ·".into(),
            });
            if out.len() >= cap {
                break;
            }
        }
    }
    if out.is_empty() {
        lines.iter().take(cap).cloned().collect()
    } else {
        out
    }
}

fn cap_store(mut lines: Vec<DiffLine>) -> Vec<DiffLine> {
    if lines.len() > STORE_CAP {
        lines.truncate(STORE_CAP);
        lines.push(DiffLine {
            op: Op::Eq,
            text: "…".into(),
        });
    }
    lines
}

fn diff_lines(old: &[&str], new: &[&str]) -> Vec<DiffLine> {
    if old.is_empty() {
        return new
            .iter()
            .map(|t| DiffLine {
                op: Op::Add,
                text: (*t).into(),
            })
            .collect();
    }
    if new.is_empty() {
        return old
            .iter()
            .map(|t| DiffLine {
                op: Op::Del,
                text: (*t).into(),
            })
            .collect();
    }
    if old.len() > DP_CAP || new.len() > DP_CAP {
        let mut v = Vec::new();
        for t in old {
            v.push(DiffLine {
                op: Op::Del,
                text: (*t).into(),
            });
        }
        for t in new {
            v.push(DiffLine {
                op: Op::Add,
                text: (*t).into(),
            });
        }
        return v;
    }
    lcs(old, new)
}

fn lcs(old: &[&str], new: &[&str]) -> Vec<DiffLine> {
    let n = old.len();
    let m = new.len();
    let w = m + 1;
    let mut dp = vec![0u16; (n + 1) * (m + 1)];
    for i in 1..=n {
        for j in 1..=m {
            let idx = i * w + j;
            dp[idx] = if old[i - 1] == new[j - 1] {
                dp[(i - 1) * w + (j - 1)].saturating_add(1)
            } else {
                dp[(i - 1) * w + j].max(dp[i * w + (j - 1)])
            };
        }
    }
    let mut out = Vec::new();
    let mut i = n;
    let mut j = m;
    while i > 0 && j > 0 {
        if old[i - 1] == new[j - 1] {
            out.push(DiffLine {
                op: Op::Eq,
                text: old[i - 1].into(),
            });
            i -= 1;
            j -= 1;
        } else if dp[(i - 1) * w + j] >= dp[i * w + (j - 1)] {
            out.push(DiffLine {
                op: Op::Del,
                text: old[i - 1].into(),
            });
            i -= 1;
        } else {
            out.push(DiffLine {
                op: Op::Add,
                text: new[j - 1].into(),
            });
            j -= 1;
        }
    }
    while i > 0 {
        out.push(DiffLine {
            op: Op::Del,
            text: old[i - 1].into(),
        });
        i -= 1;
    }
    while j > 0 {
        out.push(DiffLine {
            op: Op::Add,
            text: new[j - 1].into(),
        });
        j -= 1;
    }
    out.reverse();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_is_all_adds() {
        let d = diff_text("", "one\ntwo");
        assert!(d.iter().all(|l| l.op == Op::Add), "{d:?}");
        assert_eq!(counts(&d), (2, 0));
    }

    #[test]
    fn delete_is_all_dels() {
        let d = diff_text("one\ntwo", "");
        assert!(d.iter().all(|l| l.op == Op::Del), "{d:?}");
        assert_eq!(counts(&d), (0, 2));
    }

    #[test]
    fn mixed_edit() {
        let d = diff_text("keep\nold\nalso", "keep\nnew\nalso");
        let adds: Vec<_> = d
            .iter()
            .filter(|l| l.op == Op::Add)
            .map(|l| l.text.as_str())
            .collect();
        let dels: Vec<_> = d
            .iter()
            .filter(|l| l.op == Op::Del)
            .map(|l| l.text.as_str())
            .collect();
        assert_eq!(adds, vec!["new"]);
        assert_eq!(dels, vec!["old"]);
        assert!(d.iter().any(|l| l.op == Op::Eq && l.text == "keep"));
    }
}
