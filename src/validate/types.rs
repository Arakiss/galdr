/// How seriously a finding is taken. An `Error` always blocks an install; a `Warn`
/// blocks only under `--strict`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warn,
}

impl Severity {
    fn label(self) -> &'static str {
        match self {
            Severity::Error => "ERROR",
            Severity::Warn => "warn",
        }
    }
}

/// Which axis a finding belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Security,
    Optimization,
    Practicality,
}

impl Category {
    fn label(self) -> &'static str {
        match self {
            Category::Security => "security",
            Category::Optimization => "optimization",
            Category::Practicality => "practicality",
        }
    }
}

/// One thing the gate found in a skill.
#[derive(Debug, Clone)]
pub struct Finding {
    pub severity: Severity,
    pub category: Category,
    pub code: &'static str,
    pub message: String,
    /// 1-based line in the skill, when the finding is tied to one.
    pub line: Option<usize>,
}

/// The full result of validating one skill.
#[derive(Debug, Clone, Default)]
pub struct ValidationReport {
    pub findings: Vec<Finding>,
}

impl ValidationReport {
    fn push(
        &mut self,
        severity: Severity,
        category: Category,
        code: &'static str,
        message: impl Into<String>,
        line: Option<usize>,
    ) {
        self.findings.push(Finding {
            severity,
            category,
            code,
            message: message.into(),
            line,
        });
    }

    pub fn is_empty(&self) -> bool {
        self.findings.is_empty()
    }

    pub fn errors(&self) -> usize {
        self.findings
            .iter()
            .filter(|f| f.severity == Severity::Error)
            .count()
    }

    pub fn warnings(&self) -> usize {
        self.findings
            .iter()
            .filter(|f| f.severity == Severity::Warn)
            .count()
    }

    /// Whether anything here should stop an install. An `Error` always blocks; a
    /// `Warn` blocks only under `--strict` ("impeccable" mode).
    pub fn has_blocking(&self, strict: bool) -> bool {
        self.findings
            .iter()
            .any(|f| f.severity == Severity::Error || (strict && f.severity == Severity::Warn))
    }

    pub fn blocking_count(&self, strict: bool) -> usize {
        self.findings
            .iter()
            .filter(|f| f.severity == Severity::Error || (strict && f.severity == Severity::Warn))
            .count()
    }
}

impl fmt::Display for ValidationReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for finding in &self.findings {
            let where_ = finding
                .line
                .map(|l| format!(" (line {l})"))
                .unwrap_or_default();
            writeln!(
                f,
                "  [{}] {}/{}{} — {}",
                finding.severity.label(),
                finding.category.label(),
                finding.code,
                where_,
                finding.message
            )?;
        }
        Ok(())
    }
}

/// Context that tunes the gate for the call site.
#[derive(Debug, Clone)]
pub struct ValidationCtx {
    /// A draft skips the Practicality axis (it is scaffolding a human will finish),
    /// but keeps the full Security axis (it is a file a human is about to open).
    pub draft: bool,
    /// Under strict ("impeccable") mode, optimization and documented-danger warnings
    /// also block.
    pub strict: bool,
    /// The user's home directory, used to generalize a personal path back to `~`.
    pub home: Option<String>,
}

impl ValidationCtx {
    pub fn new(draft: bool, strict: bool) -> Self {
        Self {
            draft,
            strict,
            home: crate::paths::home_dir().map(|p| p.display().to_string()),
        }
    }
}
