//! Indented source writer with stable byte positions.

const INDENT: &str = "    ";

/// Precomputed translation from an unindented fragment into generated source.
pub(crate) struct IndentedOffsetMap {
    base: usize,
    indent_bytes: usize,
    line_count: usize,
    newline_offsets: Vec<usize>,
}

impl IndentedOffsetMap {
    pub(crate) fn new(source: &str, base: usize, indent: usize) -> Self {
        let newline_offsets = source
            .bytes()
            .enumerate()
            .filter_map(|(offset, byte)| (byte == b'\n').then_some(offset))
            .collect::<Vec<_>>();
        let line_count = if source.is_empty() {
            0
        } else {
            newline_offsets.len() + usize::from(!source.ends_with('\n'))
        };
        Self {
            base,
            indent_bytes: indent * INDENT.len(),
            line_count,
            newline_offsets,
        }
    }

    pub(crate) fn translate(&self, offset: usize) -> usize {
        if self.line_count == 0 {
            return self.base;
        }
        let preceding_newlines = self
            .newline_offsets
            .partition_point(|&newline| newline < offset);
        let prefixes = (1 + preceding_newlines).min(self.line_count);
        self.base + offset + prefixes * self.indent_bytes
    }
}

#[derive(Default)]
pub(crate) struct SourceWriter {
    source: String,
    indent: usize,
}

impl SourceWriter {
    pub(crate) fn line(&mut self, value: &str) {
        for _ in 0..self.indent {
            self.source.push_str(INDENT);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexed_offsets_match_line_prefix_insertion() {
        for source in ["", "one", "one\n", "one\ntwo", "one\n\ntwo\n"] {
            let mapping = IndentedOffsetMap::new(source, 7, 2);
            for offset in 0..=source.len() {
                assert_eq!(mapping.translate(offset), reference(source, offset, 7, 8));
            }
        }
    }

    #[allow(clippy::naive_bytecount)]
    fn reference(source: &str, offset: usize, base: usize, indent_bytes: usize) -> usize {
        if source.is_empty() {
            return base;
        }
        let total_lines = source.lines().count();
        let prefixes = (1 + source.as_bytes()[..offset.min(source.len())]
            .iter()
            .filter(|&&byte| byte == b'\n')
            .count())
        .min(total_lines);
        base + offset + prefixes * indent_bytes
    }
}
