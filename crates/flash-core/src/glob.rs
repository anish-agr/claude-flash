//! Wildcard matching for project rules.
//!
//! Deliberately small: `*` matches any run of characters (including path
//! separators) and `?` matches exactly one. Matching ignores case, and paths are
//! compared with forward slashes, so one rule works on Windows and macOS alike.

/// Normalises a filesystem path for matching: forward slashes, lower case, no
/// trailing separator.
pub fn normalize_path(path: &str) -> String {
    let mut s: String = path.replace('\\', "/").to_lowercase();
    while s.len() > 1 && s.ends_with('/') {
        s.pop();
    }
    s
}

/// The last component of a path, which is what a project is called by default.
pub fn basename(path: &str) -> &str {
    path.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or(path)
}

/// Case-insensitive wildcard match of `pattern` against the whole of `text`.
pub fn matches(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let t: Vec<char> = text.to_lowercase().chars().collect();
    // Iterative matcher with single-star backtracking: linear in practice and immune
    // to the exponential blow-up of naive recursive globbing.
    let (mut pi, mut ti) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some((pi, ti));
            pi += 1;
        } else if let Some((sp, st)) = star {
            pi = sp + 1;
            ti = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_and_wildcards() {
        assert!(matches("pricetime", "pricetime"));
        assert!(matches("price*", "pricetime"));
        assert!(matches("*time", "pricetime"));
        assert!(matches("p?icetime", "pricetime"));
        assert!(!matches("price", "pricetime"));
        assert!(!matches("?", ""));
        assert!(matches("*", ""));
    }

    #[test]
    fn star_crosses_separators() {
        assert!(matches("*/scratch/*", "c:/users/a/scratch/run-17"));
    }

    #[test]
    fn case_insensitive() {
        assert!(matches("*/GitHub/*", "C:/Users/a/github/x"));
    }

    #[test]
    fn pathological_patterns_stay_fast() {
        let text = "a".repeat(10_000);
        let pattern = format!("{}b", "*a".repeat(50));
        assert!(!matches(&pattern, &text));
    }

    #[test]
    fn normalizes_windows_paths() {
        assert_eq!(normalize_path(r"C:\Users\Anish\Repo\"), "c:/users/anish/repo");
        assert_eq!(normalize_path("/"), "/");
    }

    #[test]
    fn basename_handles_both_separators() {
        assert_eq!(basename(r"C:\work\claude-flash"), "claude-flash");
        assert_eq!(basename("/Users/a/pricetime/"), "pricetime");
        assert_eq!(basename("solo"), "solo");
    }
}
