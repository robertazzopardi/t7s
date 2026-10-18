#[derive(Debug, Clone)]
pub struct CellEditSession {
    pub row_index: usize,
    pub col_index: usize,
    pub buffer: String,
    pub pre_edit_value: String,
    pub row_snapshot: Vec<String>,
}

use crate::db::{Column, column_is_editable, sole_pk, sqlite_rowid_pk_omit};

#[derive(Debug, Clone, Copy)]
pub struct EditableCol {
    pub index: usize,
    pub editable: bool,
}

/// Table-data Browse bindings that must not type-to-overwrite.
///
/// Must include every `Char` handled by `handle_hotkeys` / navigation while on
/// table data so `handle_table_data_hotkeys` can fall through to global handlers.
pub fn is_table_data_browse_binding(c: char) -> bool {
    matches!(
        c,
        'q' | 'e'
            | 'E'
            | 't'
            | 'd'
            | '?'
            | 'b'
            | 'w'
            | 'A'
            | 'L'
            | 'o'
            | 'O'
            | 'p'
            | 'x'
            | '1'
            ..='5'
                | 'n'
                | 'N'
                | 'c'
                | 's'
                | 'r'
                | 'D'
                | 'R'
                | 'S'
                | 'C'
                | 'a'
                | 'i'
                | 'y'
                | 'Y'
                | '/'
                | ':'
                | '#'
                | ' '
                | 'j'
                | 'k'
                | 'h'
                | 'l'
                | 'g'
                | 'G'
                | '0'
                | '$'
    )
}

pub fn next_editable_column(
    cols: &[EditableCol],
    from: usize,
) -> Option<usize> {
    let editable: Vec<usize> = cols
        .iter()
        .filter(|c| c.editable)
        .map(|c| c.index)
        .collect();
    if editable.is_empty() {
        return None;
    }
    editable
        .iter()
        .find(|&&i| i > from)
        .copied()
        .or_else(|| editable.first().copied())
}

/// Fallback when column metadata is not loaded yet.
pub fn all_editable_columns(num_columns: usize) -> Vec<EditableCol> {
    (0..num_columns)
        .map(|index| EditableCol {
            index,
            editable: true,
        })
        .collect()
}

#[must_use]
pub fn editable_columns_from_meta(
    columns: &[Column],
    pk_names: &[String],
    engine_sqlite: bool,
) -> Vec<EditableCol> {
    let pk_first = sole_pk(pk_names);
    columns
        .iter()
        .enumerate()
        .map(|(index, col)| {
            let sqlite_rowid_pk =
                engine_sqlite && sqlite_rowid_pk_omit(col, pk_first);
            EditableCol {
                index,
                editable: column_is_editable(
                    col,
                    engine_sqlite,
                    sqlite_rowid_pk,
                ),
            }
        })
        .collect()
}

pub fn prev_editable_column(
    cols: &[EditableCol],
    from: usize,
) -> Option<usize> {
    let editable: Vec<usize> = cols
        .iter()
        .filter(|c| c.editable)
        .map(|c| c.index)
        .collect();
    if editable.is_empty() {
        return None;
    }
    editable
        .iter()
        .rev()
        .find(|&&i| i < from)
        .copied()
        .or_else(|| editable.last().copied())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browse_bindings_include_create_and_commit_keys() {
        for c in [
            'q', 'e', 'E', 't', 'b', 'w', 'A', 'L', 'o', 'O', 'p', 'x', '1',
            '5', 'n', 'N', 'c', 's', 'r', 'R', 'S', 'C', 'D', 'a', 'i', 'y',
            'Y', 'd', '/', ':', '#', '?', 'j', 'k', 'h', 'l', 'g', 'G', '0',
            '$',
        ] {
            assert!(is_table_data_browse_binding(c), "{c}");
        }
        assert!(!is_table_data_browse_binding('z'));
    }

    #[test]
    fn composite_sqlite_pk_stays_editable() {
        let col = |name: &str| Column {
            name: name.into(),
            data_type: "INTEGER".into(),
            is_nullable: false,
            default_value: None,
            description: None,
        };
        let cols = [col("a"), col("b")];
        let both = ["a".to_string(), "b".to_string()];
        assert!(
            editable_columns_from_meta(&cols, &both, true)
                .iter()
                .all(|c| c.editable)
        );
        let one = ["a".to_string()];
        let ec = editable_columns_from_meta(&cols, &one, true);
        assert!(!ec[0].editable && ec[1].editable);
        let mut ident = col("id");
        ident.default_value = Some("GENERATED AS IDENTITY".into());
        assert!(!editable_columns_from_meta(&[ident], &[], false)[0].editable);
        let mut auto = col("id");
        auto.default_value = Some("auto_increment".into());
        assert!(!editable_columns_from_meta(&[auto], &[], false)[0].editable);
    }

    #[test]
    fn tab_skips_non_editable_and_wraps_within_row() {
        let cols = [
            EditableCol {
                index: 0,
                editable: false,
            },
            EditableCol {
                index: 1,
                editable: true,
            },
            EditableCol {
                index: 2,
                editable: true,
            },
        ];
        assert_eq!(next_editable_column(&cols, 1), Some(2));
        assert_eq!(next_editable_column(&cols, 2), Some(1)); // wrap
        assert_eq!(prev_editable_column(&cols, 1), Some(2));
        assert_eq!(next_editable_column(&cols, 0), Some(1));
    }
}
