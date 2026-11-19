//! Santec pattern CSV in the notebook `Y/X` layout:
//!
//! ```text
//! Y/X,0,1,…,W-1
//! 0,c00,c01,…
//! …
//! H-1,…
//! ```
//!
//! Reading validates the header, size, row/column indices and every code, and
//! reports the 1-based line and column of the first problem.

use crate::frame::CodeFrame;
use std::fmt::Write as _;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CsvError {
    /// 1-based line in the file.
    pub line: usize,
    /// 1-based field in the line.
    pub column: usize,
    pub message: String,
}

impl std::fmt::Display for CsvError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "line {}, column {}: {}",
            self.line, self.column, self.message
        )
    }
}

impl std::error::Error for CsvError {}

fn err(line: usize, column: usize, message: impl Into<String>) -> CsvError {
    CsvError {
        line,
        column,
        message: message.into(),
    }
}

fn clean(field: &str) -> &str {
    let field = field.trim();
    field
        .strip_prefix('"')
        .and_then(|f| f.strip_suffix('"'))
        .unwrap_or(field)
        .trim()
}

/// Parse an integer field; pandas may write integral floats as `12.0`.
fn integer(field: &str) -> Option<i64> {
    let field = clean(field);
    if let Ok(v) = field.parse::<i64>() {
        return Some(v);
    }
    let v = field.parse::<f64>().ok()?;
    (v.is_finite() && v.fract() == 0. && v.abs() < 9e15).then_some(v as i64)
}

pub fn read_santec_csv(
    text: &str,
    width: u32,
    height: u32,
    max_code: u16,
) -> Result<CodeFrame, CsvError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = text
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty());
    let (header_index, header) = lines.next().ok_or_else(|| err(1, 1, "File is empty"))?;
    let header_line = header_index + 1;
    let mut fields = header.split(',');
    let corner = fields.next().unwrap_or("");
    if clean(corner) != "Y/X" {
        return Err(err(
            header_line,
            1,
            format!("Expected header cell \"Y/X\", found \"{}\"", clean(corner)),
        ));
    }
    let mut columns = 0u32;
    for (k, field) in fields.enumerate() {
        let column = k + 2;
        let index = integer(field).ok_or_else(|| {
            err(
                header_line,
                column,
                format!("Column index \"{}\" is not an integer", clean(field)),
            )
        })?;
        if index != i64::from(columns) {
            return Err(err(
                header_line,
                column,
                format!("Expected column index {columns}, found {index}"),
            ));
        }
        columns += 1;
    }
    if columns != width {
        return Err(err(
            header_line,
            columns as usize + 1,
            format!(
                "Expected {width} columns (0–{}), found {columns}",
                width.saturating_sub(1)
            ),
        ));
    }
    let mut data = Vec::with_capacity(width as usize * height as usize);
    let mut rows = 0u32;
    for (index, line) in lines {
        let line_no = index + 1;
        if rows == height {
            return Err(err(line_no, 1, format!("More than {height} rows")));
        }
        let mut fields = line.split(',');
        let row = fields.next().unwrap_or("");
        let row_index = integer(row).ok_or_else(|| {
            err(
                line_no,
                1,
                format!("Row index \"{}\" is not an integer", clean(row)),
            )
        })?;
        if row_index != i64::from(rows) {
            return Err(err(
                line_no,
                1,
                format!("Expected row index {rows}, found {row_index}"),
            ));
        }
        let mut count = 0u32;
        for (k, field) in fields.enumerate() {
            let column = k + 2;
            if count == width {
                return Err(err(
                    line_no,
                    column,
                    format!("Row has more than {width} values"),
                ));
            }
            let value = integer(field).ok_or_else(|| {
                err(
                    line_no,
                    column,
                    format!("Value \"{}\" is not an integer code", clean(field)),
                )
            })?;
            if !(0..=i64::from(max_code)).contains(&value) {
                return Err(err(
                    line_no,
                    column,
                    format!("Code {value} is outside 0–{max_code}"),
                ));
            }
            data.push(value as u16);
            count += 1;
        }
        if count != width {
            return Err(err(
                line_no,
                count as usize + 2,
                format!("Row {rows} has {count} values; expected {width}"),
            ));
        }
        rows += 1;
    }
    if rows != height {
        return Err(err(
            text.lines().count() + 1,
            1,
            format!(
                "Expected {height} rows (0–{}), found {rows}",
                height.saturating_sub(1)
            ),
        ));
    }
    Ok(CodeFrame {
        width,
        height,
        data,
    })
}

/// Write the notebook `Y/X` layout with `\n` line endings, as pandas does.
pub fn write_santec_csv(frame: &CodeFrame) -> String {
    let w = frame.width as usize;
    let mut out = String::with_capacity((w * 5 + 8) * (frame.height as usize + 1));
    out.push_str("Y/X");
    for x in 0..w {
        let _ = write!(out, ",{x}");
    }
    out.push('\n');
    for (y, row) in frame.data.chunks(w).enumerate() {
        let _ = write!(out, "{y}");
        for code in row {
            let _ = write!(out, ",{code}");
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let frame = CodeFrame {
            width: 3,
            height: 2,
            data: vec![0, 1, 1023, 5, 6, 7],
        };
        let text = write_santec_csv(&frame);
        assert_eq!(text, "Y/X,0,1,2\n0,0,1,1023\n1,5,6,7\n");
        assert_eq!(read_santec_csv(&text, 3, 2, 1023).unwrap(), frame);
        let crlf = text.replace('\n', "\r\n");
        assert_eq!(
            read_santec_csv(&format!("\u{feff}{crlf}"), 3, 2, 1023).unwrap(),
            frame
        );
    }

    #[test]
    fn errors_report_location() {
        let e = read_santec_csv("X/Y,0\n0,1\n", 1, 1, 1023).unwrap_err();
        assert_eq!((e.line, e.column), (1, 1));
        let e = read_santec_csv("Y/X,0,2\n", 2, 1, 1023).unwrap_err();
        assert_eq!((e.line, e.column), (1, 3));
        let e = read_santec_csv("Y/X,0,1\n0,1,1024\n", 2, 1, 1023).unwrap_err();
        assert_eq!((e.line, e.column), (2, 3));
        assert!(e.message.contains("outside"));
        let e = read_santec_csv("Y/X,0,1\n0,1,x\n", 2, 1, 1023).unwrap_err();
        assert_eq!((e.line, e.column), (2, 3));
        let e = read_santec_csv("Y/X,0,1\n1,1,2\n", 2, 1, 1023).unwrap_err();
        assert_eq!((e.line, e.column), (2, 1));
        let e = read_santec_csv("Y/X,0,1\n0,1\n", 2, 1, 1023).unwrap_err();
        assert_eq!(e.line, 2);
        let e = read_santec_csv("Y/X,0,1\n0,1,2\n", 2, 2, 1023).unwrap_err();
        assert!(e.message.contains("Expected 2 rows"));
        let e = read_santec_csv("Y/X,0,1\n0,1,-1\n", 2, 1, 1023).unwrap_err();
        assert_eq!((e.line, e.column), (2, 3));
        // pandas float formatting of integral values is accepted.
        assert!(read_santec_csv("Y/X,0,1\n0,1.0,2\n", 2, 1, 1023).is_ok());
        assert!(read_santec_csv("Y/X,0,1\n0,1.5,2\n", 2, 1, 1023).is_err());
    }
}
