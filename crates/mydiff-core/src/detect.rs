//! Choosing a comparison mode from file names and contents.

use std::path::Path;

/// Content kinds the engine knows how to compare.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// Line-based text.
    Text,
    /// Raw bytes.
    Binary,
    /// JSON document.
    Json,
    /// YAML document.
    Yaml,
    /// Delimited table; the delimiter byte (`,` or `\t`).
    Csv(u8),
}

/// How many leading bytes are inspected for binary content.
const SNIFF_LEN: usize = 8 * 1024;

/// Structured kind implied by a file extension, if any.
pub fn kind_from_extension(path: &Path) -> Option<Kind> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "json" => Some(Kind::Json),
        "yaml" | "yml" => Some(Kind::Yaml),
        "csv" => Some(Kind::Csv(b',')),
        "tsv" => Some(Kind::Csv(b'\t')),
        _ => None,
    }
}

/// Heuristic binary check on the first few KiB (NUL bytes, invalid encodings).
pub fn looks_binary(bytes: &[u8]) -> bool {
    content_inspector::inspect(&bytes[..bytes.len().min(SNIFF_LEN)]).is_binary()
}

/// Pick a mode for two files: binary if either looks binary, otherwise a
/// structured kind if both extensions agree and neither file is blank,
/// otherwise text. (A blank side, such as a new or emptied file, cannot be
/// parsed as a document, but a line diff of it is still meaningful.)
pub fn detect(left: &Path, left_bytes: &[u8], right: &Path, right_bytes: &[u8]) -> Kind {
    if looks_binary(left_bytes) || looks_binary(right_bytes) {
        return Kind::Binary;
    }
    match (kind_from_extension(left), kind_from_extension(right)) {
        (Some(a), Some(b)) if a == b && !is_blank(left_bytes) && !is_blank(right_bytes) => a,
        _ => Kind::Text,
    }
}

fn is_blank(bytes: &[u8]) -> bool {
    bytes.iter().all(u8::is_ascii_whitespace)
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::json("a.json", Some(Kind::Json))]
    #[case::upper("A.JSON", Some(Kind::Json))]
    #[case::yml("a.yml", Some(Kind::Yaml))]
    #[case::yaml("a.yaml", Some(Kind::Yaml))]
    #[case::csv("a.csv", Some(Kind::Csv(b',')))]
    #[case::tsv("a.tsv", Some(Kind::Csv(b'\t')))]
    #[case::txt("a.txt", None)]
    #[case::none("Makefile", None)]
    fn extensions(#[case] name: &str, #[case] expected: Option<Kind>) {
        assert_eq!(kind_from_extension(Path::new(name)), expected);
    }

    #[rstest]
    #[case::text_files("a.txt", b"hi\n", "b.txt", b"ho\n", Kind::Text)]
    #[case::nul_byte("a.txt", b"hi\0", "b.txt", b"ho\n", Kind::Binary)]
    #[case::both_json("a.json", b"{}", "b.json", b"[]", Kind::Json)]
    #[case::mixed_ext("a.json", b"{}", "b.txt", b"{}", Kind::Text)]
    #[case::binary_beats_ext("a.json", b"\0\x01", "b.json", b"{}", Kind::Binary)]
    #[case::empty_json_side_is_text("a.json", b"", "b.json", b"{}", Kind::Text)]
    #[case::blank_csv_side_is_text("a.csv", b"id\n", "b.csv", b" \n", Kind::Text)]
    #[case::utf8_is_text("a", "héllo wörld".as_bytes(), "b", b"x", Kind::Text)]
    fn detection(
        #[case] l: &str,
        #[case] lb: &[u8],
        #[case] r: &str,
        #[case] rb: &[u8],
        #[case] expected: Kind,
    ) {
        assert_eq!(detect(Path::new(l), lb, Path::new(r), rb), expected);
    }
}
