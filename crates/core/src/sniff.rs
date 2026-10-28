//! Dialect detection: delimiter sniffing, header heuristic.
//!
//! VisiData has no delimiter sniffing (fixed `csv_delimiter` option); this is
//! one of rustidata's deliberate improvements. We score candidate delimiters
//! (`,` `\t` `;` `|`) on field-count consistency across sample lines, with
//! quote-aware field splitting.

/// A detected (or configured) text-table dialect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dialect {
    pub delimiter: u8,
    pub quote: Option<u8>,
    pub has_header: bool,
    /// Whether the sample contained any quoted field. When false the row
    /// indexer can use the fast memchr path (no embedded newlines possible).
    pub uses_quotes: bool,
}

impl Dialect {
    pub fn csv() -> Self {
        Dialect {
            delimiter: b',',
            quote: Some(b'"'),
            has_header: true,
            uses_quotes: true,
        }
    }

    pub fn tsv() -> Self {
        Dialect {
            delimiter: b'\t',
            quote: None,
            has_header: true,
            uses_quotes: false,
        }
    }

    pub fn delimiter_char(&self) -> char {
        self.delimiter as char
    }
}

const CANDIDATES: &[u8] = b",\t;|";

/// Sniff the dialect from a sample (typically the first 64KB of the file).
/// `forced_delim` skips delimiter detection but still runs header/quote
/// detection.
pub fn sniff(sample: &[u8], forced_delim: Option<u8>) -> Dialect {
    let text = String::from_utf8_lossy(sample);
    let lines = complete_lines(&text);

    let delimiter = match forced_delim {
        Some(d) => d,
        None => detect_delimiter(&lines),
    };

    // Only a quote at the start of a field opens quoting (RFC 4180, and what
    // the csv crate does on the streaming path). A bare quote mid-field, e.g.
    // `55" screen`, is literal and must not switch on quote-aware indexing.
    let uses_quotes = lines.iter().any(|l| {
        split_quoted(l, delimiter, Some(b'"'))
            .iter()
            .any(|f| f.starts_with('"'))
    });
    let quote = uses_quotes.then_some(b'"');
    let has_header = detect_header(&lines, delimiter, quote);

    Dialect {
        delimiter,
        quote,
        has_header,
        uses_quotes,
    }
}

/// Use only lines that are certainly complete (drop a trailing partial line
/// when the sample was cut mid-file).
fn complete_lines(text: &str) -> Vec<&str> {
    let mut lines: Vec<&str> = text.lines().collect();
    if !text.ends_with('\n') && lines.len() > 1 {
        lines.pop();
    }
    lines.truncate(100);
    lines
}

fn detect_delimiter(lines: &[&str]) -> u8 {
    let mut best = (b',', f64::MIN);
    for &cand in CANDIDATES {
        let score = score_delimiter(lines, cand);
        if score > best.1 {
            best = (cand, score);
        }
    }
    // None of the usual delimiters appear: fall back to a single space, but
    // only when every line has the same field count (prose has ragged counts).
    if best.1 <= f64::MIN + 1.0 && space_is_consistent(lines) {
        return b' ';
    }
    best.0
}

fn space_is_consistent(lines: &[&str]) -> bool {
    let mut counts = lines
        .iter()
        .filter(|l| !l.is_empty())
        .map(|l| l.split(' ').count());
    match counts.next() {
        Some(first) if first > 1 => counts.all(|c| c == first),
        _ => false,
    }
}

/// Score: high when the delimiter yields the same field count (>1) on every
/// line; penalized by variance; small bonus for more fields.
fn score_delimiter(lines: &[&str], delim: u8) -> f64 {
    let counts: Vec<usize> = lines
        .iter()
        .filter(|l| !l.is_empty())
        .map(|l| split_quoted(l, delim, Some(b'"')).len())
        .collect();
    if counts.is_empty() {
        return f64::MIN;
    }
    let n = counts.len() as f64;
    let mean = counts.iter().sum::<usize>() as f64 / n;
    if mean <= 1.0 {
        // delimiter never appears: useless
        return f64::MIN + 1.0;
    }
    let var = counts.iter().map(|&c| (c as f64 - mean).powi(2)).sum::<f64>() / n;
    // consistency dominates; field count breaks ties
    let consistency = (counts.iter().filter(|&&c| c as f64 == mean).count() as f64) / n;
    consistency * 100.0 - var * 10.0 + mean.min(30.0)
}

/// Split one line on `delim`, honoring double-quote quoting. A quote opens a
/// quoted section only at the start of a field; elsewhere it is literal.
/// Inside a quoted section a doubled quote is an escaped quote.
pub fn split_quoted<'a>(line: &'a str, delim: u8, quote: Option<u8>) -> Vec<&'a str> {
    let bytes = line.as_bytes();
    let mut fields = Vec::new();
    let mut start = 0usize;
    let mut in_quotes = false;
    let mut field_start = true;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if in_quotes {
            if Some(b) == quote {
                if bytes.get(i + 1).copied() == quote {
                    i += 2;
                    continue;
                }
                in_quotes = false;
            }
        } else if b == delim {
            fields.push(&line[start..i]);
            start = i + 1;
            field_start = true;
            i += 1;
            continue;
        } else if field_start && Some(b) == quote {
            in_quotes = true;
        }
        field_start = false;
        i += 1;
    }
    fields.push(&line[start..]);
    fields
}

/// Header heuristic, in the spirit of python csv.Sniffer: if the first row's
/// fields are all non-numeric but later rows have numeric fields in the same
/// positions, the first row is a header. Also: header fields are non-empty and
/// distinct.
fn detect_header(lines: &[&str], delim: u8, quote: Option<u8>) -> bool {
    if lines.len() < 2 {
        return true; // default to header like visidata
    }
    let first = split_quoted(lines[0], delim, quote);
    if first.iter().any(|f| f.trim().is_empty()) {
        return false;
    }
    // duplicate names suggest data, not a header
    let mut seen = std::collections::HashSet::new();
    if !first.iter().all(|f| seen.insert(f.trim())) {
        return false;
    }
    let first_numeric: Vec<bool> = first.iter().map(|f| is_numericish(f)).collect();
    if first_numeric.iter().any(|&b| b) {
        return false; // numbers in row 1 => data
    }
    // compare against a few body rows: any column that is numeric in the body
    // but text in row 1 is strong evidence of a header
    let mut evidence = 0;
    for line in lines.iter().skip(1).take(20) {
        let fields = split_quoted(line, delim, quote);
        for (i, f) in fields.iter().enumerate() {
            if i < first_numeric.len() && !first_numeric[i] && is_numericish(f) {
                evidence += 1;
            }
        }
    }
    if evidence > 0 {
        return true;
    }
    // all-text data: fall back to "first row looks like labels" (distinct,
    // non-empty), which we already checked
    true
}

fn is_numericish(s: &str) -> bool {
    let t = s.trim();
    !t.is_empty() && crate::value::parse_float_str(t).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_comma() {
        let d = sniff(b"a,b,c\n1,2,3\n4,5,6\n", None);
        assert_eq!(d.delimiter, b',');
        assert!(d.has_header);
    }

    #[test]
    fn sniffs_tab() {
        let d = sniff(b"a\tb\tc\n1\t2\t3\n", None);
        assert_eq!(d.delimiter, b'\t');
    }

    #[test]
    fn sniffs_space_when_no_other_delimiter() {
        let d = sniff(
            b"0 0 1021440 chr1_0_1021440\n1 1021440 1167918 chr1_1021440_1167918\n",
            None,
        );
        assert_eq!(d.delimiter, b' ');
        assert!(!d.has_header);
    }

    #[test]
    fn ragged_prose_not_split_on_space() {
        let d = sniff(b"hello world\nthis is a line\nanother\n", None);
        assert_eq!(d.delimiter, b',');
    }

    #[test]
    fn sniffs_semicolon_with_commas_in_fields() {
        let d = sniff(b"name;desc\nfoo;a, b, and c\nbar;d, e\n", None);
        assert_eq!(d.delimiter, b';');
    }

    #[test]
    fn sniffs_pipe() {
        let d = sniff(b"a|b|c\n1|2|3\n4|5|6\n", None);
        assert_eq!(d.delimiter, b'|');
    }

    #[test]
    fn quoted_commas_dont_confuse() {
        let d = sniff(b"a,b\n\"x, y, z\",2\n\"p, q\",4\n", None);
        assert_eq!(d.delimiter, b',');
        assert!(d.uses_quotes);
    }

    #[test]
    fn no_header_when_first_row_numeric() {
        let d = sniff(b"1,2,3\n4,5,6\n", None);
        assert!(!d.has_header);
    }

    #[test]
    fn header_when_text_over_numbers() {
        let d = sniff(b"name,age\nalice,30\nbob,25\n", None);
        assert!(d.has_header);
    }

    #[test]
    fn forced_delimiter_respected() {
        let d = sniff(b"a,b;c\n1,2;3\n", Some(b';'));
        assert_eq!(d.delimiter, b';');
    }

    #[test]
    fn split_quoted_basic() {
        assert_eq!(split_quoted("a,b,c", b',', Some(b'"')), vec!["a", "b", "c"]);
        assert_eq!(
            split_quoted("\"a,b\",c", b',', Some(b'"')),
            vec!["\"a,b\"", "c"]
        );
        assert_eq!(split_quoted("", b',', None), vec![""]);
    }

    #[test]
    fn split_quoted_mid_field_quote_is_literal() {
        assert_eq!(
            split_quoted("TV\t55\" screen\tx", b'\t', Some(b'"')),
            vec!["TV", "55\" screen", "x"]
        );
        assert_eq!(
            split_quoted("\"a\"\"b\tc\"\td", b'\t', Some(b'"')),
            vec!["\"a\"\"b\tc\"", "d"]
        );
    }

    #[test]
    fn mid_field_quotes_dont_enable_quoting() {
        let d = sniff(b"item\tsize\nTV\t55\" screen\nphone\t6\"\n", None);
        assert_eq!(d.delimiter, b'\t');
        assert!(!d.uses_quotes);
        let d = sniff(b"a\tb\n\"x\ty\"\t2\n", None);
        assert!(d.uses_quotes);
    }

    #[test]
    fn partial_trailing_line_dropped() {
        // last line truncated mid-field should not skew detection
        let d = sniff(b"a,b,c\n1,2,3\n4,5", None);
        assert_eq!(d.delimiter, b',');
    }
}
