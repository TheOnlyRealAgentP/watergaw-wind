//! NOAA `.idx` inventory files: which byte range holds which field.
//!
//! Each line is `n:offset:d=YYYYMMDDHH:VAR:LEVEL:FORECAST:`; a field runs from its
//! offset to one byte before the next line's offset (or the end of the file).

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub var: String,
    pub level: String,
    pub forecast: String,
    pub start: u64,
    /// Inclusive end byte, or None for the last field in the file.
    pub end: Option<u64>,
}

pub fn parse(text: &str) -> Vec<Entry> {
    let rows: Vec<(u64, Vec<&str>)> = text
        .lines()
        .filter_map(|l| {
            let parts: Vec<&str> = l.split(':').collect();
            (parts.len() >= 6).then(|| parts[1].parse::<u64>().ok().map(|o| (o, parts)))?
        })
        .collect();
    rows.iter()
        .enumerate()
        .map(|(i, (start, p))| Entry {
            var: p[3].to_string(),
            level: p[4].to_string(),
            forecast: p[5].to_string(),
            start: *start,
            end: rows.get(i + 1).map(|(next, _)| next - 1),
        })
        .collect()
}

/// First entry matching a variable and level, e.g. ("UGRD", "10 m above ground").
pub fn find<'a>(entries: &'a [Entry], var: &str, level: &str) -> Option<&'a Entry> {
    entries.iter().find(|e| e.var == var && e.level == level)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_ranges() {
        let text = "584:414000000:d=2026111612:TMP:2 m above ground:anl:\n585:415035988:d=2026111612:UGRD:10 m above ground:anl:\n586:416002701:d=2026111612:VGRD:10 m above ground:anl:\n";
        let e = parse(text);
        let u = find(&e, "UGRD", "10 m above ground").unwrap();
        assert_eq!((u.start, u.end), (415035988, Some(416002700)));
        assert_eq!(find(&e, "VGRD", "10 m above ground").unwrap().end, None);
    }
}
