//! Indented source writer with stable byte positions.

#[derive(Default)]
pub(crate) struct SourceWriter {
    source: String,
    indent: usize,
}

impl SourceWriter {
    pub(crate) fn line(&mut self, value: &str) {
        for _ in 0..self.indent {
            self.source.push_str("    ");
        }
        self.source.push_str(value);
        self.source.push('\n');
    }

    pub(crate) fn blank(&mut self) {
        self.source.push('\n');
    }

    pub(crate) fn indent(&mut self) {
        self.indent += 1;
    }

    pub(crate) fn dedent(&mut self) {
        self.indent = self.indent.saturating_sub(1);
    }

    pub(crate) fn position(&self) -> usize {
        self.source.len()
    }

    pub(crate) fn push_source(&mut self, value: &str) -> usize {
        let start = self.position();
        for line in value.lines() {
            self.line(line);
        }
        start
    }

    pub(crate) fn finish(self) -> String {
        self.source
    }
}
