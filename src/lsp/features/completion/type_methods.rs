//! Type method tables for Rholang built-in types
//!
//! This module defines the methods available on Rholang's built-in collection types
//! (List, Map, Set, String, Int) for type-aware code completion.

use super::dictionary::SymbolMetadata;
use tower_lsp::lsp_types::CompletionItemKind;
use std::collections::HashMap;

/// Get all methods for a given type name
pub fn get_type_methods(type_name: &str) -> Vec<SymbolMetadata> {
    match type_name.to_lowercase().as_str() {
        "list" => list_methods(),
        "map" => map_methods(),
        "set" => set_methods(),
        "string" => string_methods(),
        "int" | "integer" => int_methods(),
        "bytearray" | "byte_array" => bytearray_methods(),
        "pathmap" | "path_map" => pathmap_methods(),
        "tuple" => tuple_methods(),
        _ => vec![],
    }
}

/// Returns all type method tables as a HashMap for quick lookup
pub fn all_type_methods() -> HashMap<String, Vec<SymbolMetadata>> {
    let mut methods = HashMap::new();
    methods.insert("List".to_string(), list_methods());
    methods.insert("Map".to_string(), map_methods());
    methods.insert("Set".to_string(), set_methods());
    methods.insert("String".to_string(), string_methods());
    methods.insert("Int".to_string(), int_methods());
    methods.insert("ByteArray".to_string(), bytearray_methods());
    methods.insert("PathMap".to_string(), pathmap_methods());
    methods.insert("Tuple".to_string(), tuple_methods());
    methods
}

/// List type methods (from Rholang interpreter)
fn list_methods() -> Vec<SymbolMetadata> {
    vec![
        SymbolMetadata {
            name: "length".to_string(),
            kind: CompletionItemKind::METHOD,
            documentation: Some("Returns the number of elements in the list".to_string()),
            signature: Some("() -> Int".to_string()),
            reference_count: 0,
        },
        SymbolMetadata {
            name: "nth".to_string(),
            kind: CompletionItemKind::METHOD,
            documentation: Some("Returns the element at the specified index".to_string()),
            signature: Some("(Int) -> T".to_string()),
            reference_count: 0,
        },
        SymbolMetadata {
            name: "slice".to_string(),
            kind: CompletionItemKind::METHOD,
            documentation: Some("Returns a sublist from start to end index".to_string()),
            signature: Some("(Int, Int) -> List[T]".to_string()),
            reference_count: 0,
        },
        SymbolMetadata {
            name: "take".to_string(),
            kind: CompletionItemKind::METHOD,
            documentation: Some("Returns the first n elements of the list".to_string()),
            signature: Some("(Int) -> List[T]".to_string()),
            reference_count: 0,
        },
        SymbolMetadata {
            name: "toSet".to_string(),
            kind: CompletionItemKind::METHOD,
            documentation: Some("Converts the list to a set".to_string()),
            signature: Some("() -> Set[T]".to_string()),
            reference_count: 0,
        },
        SymbolMetadata {
            name: "toList".to_string(),
            kind: CompletionItemKind::METHOD,
            documentation: Some("Returns the list (identity function)".to_string()),
            signature: Some("() -> List[T]".to_string()),
            reference_count: 0,
        },
    ]
}

/// Map type methods
fn map_methods() -> Vec<SymbolMetadata> {
    vec![
        SymbolMetadata {
            name: "get".to_string(),
            kind: CompletionItemKind::METHOD,
            documentation: Some("Returns the value associated with a key".to_string()),
            signature: Some("(K) -> V".to_string()),
            reference_count: 0,
        },
        SymbolMetadata {
            name: "getOrElse".to_string(),
            kind: CompletionItemKind::METHOD,
            documentation: Some("Returns the value for a key, or default if not found".to_string()),
            signature: Some("(K, V) -> V".to_string()),
            reference_count: 0,
        },
        SymbolMetadata {
            name: "set".to_string(),
            kind: CompletionItemKind::METHOD,
            documentation: Some("Returns a new map with the key-value pair added".to_string()),
            signature: Some("(K, V) -> Map[K, V]".to_string()),
            reference_count: 0,
        },
        SymbolMetadata {
            name: "delete".to_string(),
            kind: CompletionItemKind::METHOD,
            documentation: Some("Returns a new map with the key removed".to_string()),
            signature: Some("(K) -> Map[K, V]".to_string()),
            reference_count: 0,
        },
        SymbolMetadata {
            name: "contains".to_string(),
            kind: CompletionItemKind::METHOD,
            documentation: Some("Checks if the map contains a key".to_string()),
            signature: Some("(K) -> Bool".to_string()),
            reference_count: 0,
        },
        SymbolMetadata {
            name: "keys".to_string(),
            kind: CompletionItemKind::METHOD,
            documentation: Some("Returns a set of all keys in the map".to_string()),
            signature: Some("() -> Set[K]".to_string()),
            reference_count: 0,
        },
        SymbolMetadata {
            name: "size".to_string(),
            kind: CompletionItemKind::METHOD,
            documentation: Some("Returns the number of key-value pairs in the map".to_string()),
            signature: Some("() -> Int".to_string()),
            reference_count: 0,
        },
        SymbolMetadata {
            name: "union".to_string(),
            kind: CompletionItemKind::METHOD,
            documentation: Some("Returns the union of two maps".to_string()),
            signature: Some("(Map[K, V]) -> Map[K, V]".to_string()),
            reference_count: 0,
        },
        SymbolMetadata {
            name: "diff".to_string(),
            kind: CompletionItemKind::METHOD,
            documentation: Some("Returns the difference between two maps".to_string()),
            signature: Some("(Map[K, V]) -> Map[K, V]".to_string()),
            reference_count: 0,
        },
        SymbolMetadata {
            name: "toList".to_string(),
            kind: CompletionItemKind::METHOD,
            documentation: Some("Converts the map to a list of tuples".to_string()),
            signature: Some("() -> List[(K, V)]".to_string()),
            reference_count: 0,
        },
        SymbolMetadata {
            name: "toSet".to_string(),
            kind: CompletionItemKind::METHOD,
            documentation: Some("Converts the map to a set of tuples".to_string()),
            signature: Some("() -> Set[(K, V)]".to_string()),
            reference_count: 0,
        },
        SymbolMetadata {
            name: "toMap".to_string(),
            kind: CompletionItemKind::METHOD,
            documentation: Some("Returns the map (identity function)".to_string()),
            signature: Some("() -> Map[K, V]".to_string()),
            reference_count: 0,
        },
    ]
}

/// Set type methods (from Rholang interpreter)
fn set_methods() -> Vec<SymbolMetadata> {
    vec![
        SymbolMetadata {
            name: "contains".to_string(),
            kind: CompletionItemKind::METHOD,
            documentation: Some("Checks if the set contains an element".to_string()),
            signature: Some("(T) -> Bool".to_string()),
            reference_count: 0,
        },
        SymbolMetadata {
            name: "add".to_string(),
            kind: CompletionItemKind::METHOD,
            documentation: Some("Returns a new set with the element added".to_string()),
            signature: Some("(T) -> Set[T]".to_string()),
            reference_count: 0,
        },
        SymbolMetadata {
            name: "delete".to_string(),
            kind: CompletionItemKind::METHOD,
            documentation: Some("Returns a new set with the element removed".to_string()),
            signature: Some("(T) -> Set[T]".to_string()),
            reference_count: 0,
        },
        SymbolMetadata {
            name: "union".to_string(),
            kind: CompletionItemKind::METHOD,
            documentation: Some("Returns the union of two sets".to_string()),
            signature: Some("(Set[T]) -> Set[T]".to_string()),
            reference_count: 0,
        },
        SymbolMetadata {
            name: "diff".to_string(),
            kind: CompletionItemKind::METHOD,
            documentation: Some("Returns the difference between two sets".to_string()),
            signature: Some("(Set[T]) -> Set[T]".to_string()),
            reference_count: 0,
        },
        SymbolMetadata {
            name: "intersection".to_string(),
            kind: CompletionItemKind::METHOD,
            documentation: Some("Returns the intersection of two sets".to_string()),
            signature: Some("(Set[T]) -> Set[T]".to_string()),
            reference_count: 0,
        },
        SymbolMetadata {
            name: "size".to_string(),
            kind: CompletionItemKind::METHOD,
            documentation: Some("Returns the number of elements in the set".to_string()),
            signature: Some("() -> Int".to_string()),
            reference_count: 0,
        },
        SymbolMetadata {
            name: "toList".to_string(),
            kind: CompletionItemKind::METHOD,
            documentation: Some("Converts the set to a list".to_string()),
            signature: Some("() -> List[T]".to_string()),
            reference_count: 0,
        },
        SymbolMetadata {
            name: "toSet".to_string(),
            kind: CompletionItemKind::METHOD,
            documentation: Some("Returns the set (identity function)".to_string()),
            signature: Some("() -> Set[T]".to_string()),
            reference_count: 0,
        },
    ]
}

/// String type methods (from Rholang interpreter)
fn string_methods() -> Vec<SymbolMetadata> {
    vec![
        SymbolMetadata {
            name: "length".to_string(),
            kind: CompletionItemKind::METHOD,
            documentation: Some("Returns the length of the string".to_string()),
            signature: Some("() -> Int".to_string()),
            reference_count: 0,
        },
        SymbolMetadata {
            name: "slice".to_string(),
            kind: CompletionItemKind::METHOD,
            documentation: Some("Returns a substring from start to end index".to_string()),
            signature: Some("(Int, Int) -> String".to_string()),
            reference_count: 0,
        },
        SymbolMetadata {
            name: "toUtf8Bytes".to_string(),
            kind: CompletionItemKind::METHOD,
            documentation: Some("Converts the string to UTF-8 bytes".to_string()),
            signature: Some("() -> ByteArray".to_string()),
            reference_count: 0,
        },
        SymbolMetadata {
            name: "hexToBytes".to_string(),
            kind: CompletionItemKind::METHOD,
            documentation: Some("Converts a hexadecimal string to bytes".to_string()),
            signature: Some("() -> ByteArray".to_string()),
            reference_count: 0,
        },
        SymbolMetadata {
            name: "toString".to_string(),
            kind: CompletionItemKind::METHOD,
            documentation: Some("Returns the string (identity function)".to_string()),
            signature: Some("() -> String".to_string()),
            reference_count: 0,
        },
    ]
}

/// Int type methods (from Rholang interpreter)
fn int_methods() -> Vec<SymbolMetadata> {
    vec![
        SymbolMetadata {
            name: "toByteArray".to_string(),
            kind: CompletionItemKind::METHOD,
            documentation: Some("Converts the integer to a byte array".to_string()),
            signature: Some("() -> ByteArray".to_string()),
            reference_count: 0,
        },
    ]
}

/// ByteArray type methods (from Rholang interpreter)
fn bytearray_methods() -> Vec<SymbolMetadata> {
    vec![
        SymbolMetadata {
            name: "toByteArray".to_string(),
            kind: CompletionItemKind::METHOD,
            documentation: Some("Returns the byte array (identity function)".to_string()),
            signature: Some("() -> ByteArray".to_string()),
            reference_count: 0,
        },
        SymbolMetadata {
            name: "bytesToHex".to_string(),
            kind: CompletionItemKind::METHOD,
            documentation: Some("Converts the byte array to a hexadecimal string".to_string()),
            signature: Some("() -> String".to_string()),
            reference_count: 0,
        },
        SymbolMetadata {
            name: "nth".to_string(),
            kind: CompletionItemKind::METHOD,
            documentation: Some("Returns the byte at the specified index".to_string()),
            signature: Some("(Int) -> Int".to_string()),
            reference_count: 0,
        },
        SymbolMetadata {
            name: "length".to_string(),
            kind: CompletionItemKind::METHOD,
            documentation: Some("Returns the number of bytes in the array".to_string()),
            signature: Some("() -> Int".to_string()),
            reference_count: 0,
        },
        SymbolMetadata {
            name: "slice".to_string(),
            kind: CompletionItemKind::METHOD,
            documentation: Some("Returns a sub-array from start to end index".to_string()),
            signature: Some("(Int, Int) -> ByteArray".to_string()),
            reference_count: 0,
        },
    ]
}

/// PathMap type methods (from Rholang interpreter)
fn pathmap_methods() -> Vec<SymbolMetadata> {
    vec![
        SymbolMetadata {
            name: "union".to_string(),
            kind: CompletionItemKind::METHOD,
            documentation: Some("Returns the union of two path maps".to_string()),
            signature: Some("(PathMap) -> PathMap".to_string()),
            reference_count: 0,
        },
        SymbolMetadata {
            name: "diff".to_string(),
            kind: CompletionItemKind::METHOD,
            documentation: Some("Returns the difference between two path maps".to_string()),
            signature: Some("(PathMap) -> PathMap".to_string()),
            reference_count: 0,
        },
        SymbolMetadata {
            name: "intersection".to_string(),
            kind: CompletionItemKind::METHOD,
            documentation: Some("Returns the intersection of two path maps".to_string()),
            signature: Some("(PathMap) -> PathMap".to_string()),
            reference_count: 0,
        },
        SymbolMetadata {
            name: "restriction".to_string(),
            kind: CompletionItemKind::METHOD,
            documentation: Some("Returns the restriction of path map by another path map".to_string()),
            signature: Some("(PathMap) -> PathMap".to_string()),
            reference_count: 0,
        },
        SymbolMetadata {
            name: "dropHead".to_string(),
            kind: CompletionItemKind::METHOD,
            documentation: Some("Removes the first path segment from each key in the path map".to_string()),
            signature: Some("() -> PathMap".to_string()),
            reference_count: 0,
        },
        SymbolMetadata {
            name: "run".to_string(),
            kind: CompletionItemKind::METHOD,
            documentation: Some("Runs MeTTa code compiled in one path map using accumulated state from another".to_string()),
            signature: Some("(PathMap) -> PathMap".to_string()),
            reference_count: 0,
        },
    ]
}

/// Tuple type methods (from Rholang interpreter)
fn tuple_methods() -> Vec<SymbolMetadata> {
    vec![
        SymbolMetadata {
            name: "nth".to_string(),
            kind: CompletionItemKind::METHOD,
            documentation: Some("Returns the element at the specified index".to_string()),
            signature: Some("(Int) -> T".to_string()),
            reference_count: 0,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_methods() {
        let methods = list_methods();
        assert!(!methods.is_empty());
        assert!(methods.iter().any(|m| m.name == "length"));
        assert!(methods.iter().any(|m| m.name == "nth"));
    }

    #[test]
    fn test_map_methods() {
        let methods = map_methods();
        assert!(!methods.is_empty());
        assert!(methods.iter().any(|m| m.name == "get"));
        assert!(methods.iter().any(|m| m.name == "set"));
    }

    #[test]
    fn test_get_type_methods() {
        let list_methods = get_type_methods("List");
        assert!(!list_methods.is_empty());

        let unknown_methods = get_type_methods("UnknownType");
        assert!(unknown_methods.is_empty());
    }

    #[test]
    fn test_all_type_methods() {
        let all_methods = all_type_methods();
        assert_eq!(all_methods.len(), 8); // List, Map, Set, String, Int, ByteArray, PathMap, Tuple
        assert!(all_methods.contains_key("List"));
        assert!(all_methods.contains_key("Map"));
        assert!(all_methods.contains_key("ByteArray"));
        assert!(all_methods.contains_key("PathMap"));
        assert!(all_methods.contains_key("Tuple"));
    }
}
