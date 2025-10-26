use std::any::Any;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use super::printer::PrettyPrinter;

/// Trait for formatting types into a JSON-like string representation.
pub trait JsonStringFormatter {
    /// Formats self into a JSON-like string using the provided PrettyPrinter.
    fn format_json_string(&self, p: &PrettyPrinter);
}

// Primitive type implementations
impl JsonStringFormatter for bool {
    fn format_json_string(&self, p: &PrettyPrinter) {
        p.append(&self.to_string());
    }
}

impl JsonStringFormatter for i8 {
    fn format_json_string(&self, p: &PrettyPrinter) {
        p.append(&self.to_string());
    }
}

impl JsonStringFormatter for i16 {
    fn format_json_string(&self, p: &PrettyPrinter) {
        p.append(&self.to_string());
    }
}

impl JsonStringFormatter for i32 {
    fn format_json_string(&self, p: &PrettyPrinter) {
        p.append(&self.to_string());
    }
}

impl JsonStringFormatter for i64 {
    fn format_json_string(&self, p: &PrettyPrinter) {
        p.append(&self.to_string());
    }
}

impl JsonStringFormatter for i128 {
    fn format_json_string(&self, p: &PrettyPrinter) {
        p.append(&self.to_string());
    }
}

impl JsonStringFormatter for isize {
    fn format_json_string(&self, p: &PrettyPrinter) {
        p.append(&self.to_string());
    }
}

impl JsonStringFormatter for u8 {
    fn format_json_string(&self, p: &PrettyPrinter) {
        p.append(&self.to_string());
    }
}

impl JsonStringFormatter for u16 {
    fn format_json_string(&self, p: &PrettyPrinter) {
        p.append(&self.to_string());
    }
}

impl JsonStringFormatter for u32 {
    fn format_json_string(&self, p: &PrettyPrinter) {
        p.append(&self.to_string());
    }
}

impl JsonStringFormatter for u64 {
    fn format_json_string(&self, p: &PrettyPrinter) {
        p.append(&self.to_string());
    }
}

impl JsonStringFormatter for u128 {
    fn format_json_string(&self, p: &PrettyPrinter) {
        p.append(&self.to_string());
    }
}

impl JsonStringFormatter for usize {
    fn format_json_string(&self, p: &PrettyPrinter) {
        p.append(&self.to_string());
    }
}

impl JsonStringFormatter for f32 {
    fn format_json_string(&self, p: &PrettyPrinter) {
        p.append(&self.to_string());
    }
}

impl JsonStringFormatter for f64 {
    fn format_json_string(&self, p: &PrettyPrinter) {
        p.append(&self.to_string());
    }
}

impl JsonStringFormatter for char {
    fn format_json_string(&self, p: &PrettyPrinter) {
        p.escape_json_string(&self.to_string());
    }
}

impl JsonStringFormatter for String {
    fn format_json_string(&self, p: &PrettyPrinter) {
        p.escape_json_string(self);
    }
}

impl JsonStringFormatter for () {
    fn format_json_string(&self, p: &PrettyPrinter) {
        p.append("'()");
    }
}

// Collection type implementations with pretty_print support
impl<T: JsonStringFormatter + Any + Send + Sync> JsonStringFormatter for Vec<T> {
    fn format_json_string(&self, p: &PrettyPrinter) {
        if self.is_empty() {
            p.append("[]");
            return;
        }
        if p.pretty_print {
            p.append("[");
            let align_col = *p.current_column.borrow();
            for (i, item) in self.iter().enumerate() {
                if i > 0 {
                    p.append("\n");
                    p.append(&" ".repeat(align_col));
                }
                item.format_json_string(p);
            }
            p.append("]");
        } else {
            p.append("[");
            for (i, item) in self.iter().enumerate() {
                if i > 0 {
                    p.append(" ");
                }
                item.format_json_string(p);
            }
            p.append("]");
        }
    }
}

impl<T: JsonStringFormatter + Any + Send + Sync> JsonStringFormatter for HashMap<String, T> {
    fn format_json_string(&self, p: &PrettyPrinter) {
        if self.is_empty() {
            p.append("{}");
            return;
        }
        let mut sorted: Vec<_> = self.iter().collect();
        sorted.sort_by_key(|&(k, _)| k);
        if p.pretty_print {
            p.append("{");
            let align_col = *p.current_column.borrow();
            for (i, (key, value)) in sorted.iter().enumerate() {
                if i > 0 {
                    p.append("\n");
                    p.append(&" ".repeat(align_col));
                }
                p.append(":");
                p.append(key);
                p.append(" ");
                value.format_json_string(p);
            }
            p.append("}");
        } else {
            p.append("{");
            for (i, (key, value)) in sorted.iter().enumerate() {
                if i > 0 {
                    p.append(",");
                }
                p.append(":");
                p.append(key);
                p.append(" ");
                value.format_json_string(p);
            }
            p.append("}");
        }
    }
}

impl<T: JsonStringFormatter + Any + Send + Sync> JsonStringFormatter for BTreeMap<String, T> {
    fn format_json_string(&self, p: &PrettyPrinter) {
        if self.is_empty() {
            p.append("{}");
            return;
        }
        if p.pretty_print {
            p.append("{");
            let align_col = *p.current_column.borrow();
            for (i, (key, value)) in self.iter().enumerate() {
                if i > 0 {
                    p.append("\n");
                    p.append(&" ".repeat(align_col));
                }
                p.append(":");
                p.append(key);
                p.append(" ");
                value.format_json_string(p);
            }
            p.append("}");
        } else {
            p.append("{");
            for (i, (key, value)) in self.iter().enumerate() {
                if i > 0 {
                    p.append(",");
                }
                p.append(":");
                p.append(key);
                p.append(" ");
                value.format_json_string(p);
            }
            p.append("}");
        }
    }
}

impl<T: JsonStringFormatter + Any + Send + Sync> JsonStringFormatter for Option<T> {
    fn format_json_string(&self, p: &PrettyPrinter) {
        match self {
            Some(value) => value.format_json_string(p),
            None => p.append("nil"),
        }
    }
}

/// Formats a metadata value into a JSON-like string, handling various types.
///
/// # Arguments
/// * p - The PrettyPrinter instance to append to.
/// * value - The metadata value to format.
pub(super) fn format_json_string(p: &PrettyPrinter, value: &Arc<dyn Any + Send + Sync>) {
    macro_rules! try_format {
        ($($t:ty),*) => {
            $(
                if let Some(val) = value.downcast_ref::<$t>() {
                    val.format_json_string(p);
                    return;
                }
            )*
        };
    }
    try_format!(
        String,
        usize, u128, u64, u32, u16, u8,
        isize, i128, i64, i32, i16, i8,
        f64, f32,
        char,
        bool
    );

    // Handle collection types
    if let Some(map) = value.downcast_ref::<BTreeMap<String, i32>>() {
        map.format_json_string(p);
    } else if let Some(map) = value.downcast_ref::<HashMap<String, i32>>() {
        map.format_json_string(p);
    } else if let Some(vec) = value.downcast_ref::<Vec<i32>>() {
        vec.format_json_string(p);
    } else if let Some(set) = value.downcast_ref::<HashSet<i32>>() {
        let mut vec: Vec<i32> = set.iter().cloned().collect();
        vec.sort();
        if p.pretty_print {
            p.append("#{");
            let align_col = *p.current_column.borrow();
            for (i, item) in vec.iter().enumerate() {
                if i > 0 {
                    p.append("\n");
                    p.append(&" ".repeat(align_col));
                }
                item.format_json_string(p);
            }
            p.append("}");
        } else {
            p.append("#{");
            for (i, item) in vec.iter().enumerate() {
                if i > 0 {
                    p.append(" ");
                }
                item.format_json_string(p);
            }
            p.append("}");
        }
    } else if let Some(set) = value.downcast_ref::<HashSet<String>>() {
        let mut vec: Vec<String> = set.iter().cloned().collect();
        vec.sort();
        if p.pretty_print {
            p.append("#{");
            let align_col = *p.current_column.borrow();
            for (i, item) in vec.iter().enumerate() {
                if i > 0 {
                    p.append("\n");
                    p.append(&" ".repeat(align_col));
                }
                item.format_json_string(p);
            }
            p.append("}");
        } else {
            p.append("#{");
            for (i, item) in vec.iter().enumerate() {
                if i > 0 {
                    p.append(" ");
                }
                item.format_json_string(p);
            }
            p.append("}");
        }
    } else if let Some(opt) = value.downcast_ref::<Option<i32>>() {
        opt.format_json_string(p);
    } else if let Some(unit) = value.downcast_ref::<()>() {
        unit.format_json_string(p);
    } else {
        // Fallback for unhandled types
        p.escape_json_string(&format!("{:?}", value));
    }
}
