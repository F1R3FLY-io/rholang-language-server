//! Serde helpers for persistent cache serialization (Phase B-3)
//!
//! This module provides custom serialization/deserialization for types that
//! don't implement Serialize/Deserialize by default, particularly Arc<T>.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::sync::Arc;

/// Serialize Arc<T> by serializing the inner value
pub fn serialize_arc<S, T>(arc: &Arc<T>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    T: Serialize,
{
    arc.as_ref().serialize(serializer)
}

/// Deserialize Arc<T> by deserializing the inner value and wrapping in Arc
pub fn deserialize_arc<'de, D, T>(deserializer: D) -> Result<Arc<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    let value = T::deserialize(deserializer)?;
    Ok(Arc::new(value))
}

/// Serialize Vec<Arc<T>> by serializing each inner value
pub fn serialize_arc_vec<S, T>(vec: &Vec<Arc<T>>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    T: Serialize,
{
    use serde::ser::SerializeSeq;
    let mut seq = serializer.serialize_seq(Some(vec.len()))?;
    for item in vec {
        seq.serialize_element(item.as_ref())?;
    }
    seq.end()
}

/// Deserialize Vec<Arc<T>> by deserializing each value and wrapping in Arc
pub fn deserialize_arc_vec<'de, D, T>(deserializer: D) -> Result<Vec<Arc<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    let vec: Vec<T> = Vec::deserialize(deserializer)?;
    Ok(vec.into_iter().map(Arc::new).collect())
}

/// Serialize Option<Arc<T>>
pub fn serialize_option_arc<S, T>(opt: &Option<Arc<T>>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    T: Serialize,
{
    match opt {
        Some(arc) => serializer.serialize_some(arc.as_ref()),
        None => serializer.serialize_none(),
    }
}

/// Deserialize Option<Arc<T>>
pub fn deserialize_option_arc<'de, D, T>(deserializer: D) -> Result<Option<Arc<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    let opt: Option<T> = Option::deserialize(deserializer)?;
    Ok(opt.map(Arc::new))
}

/// Serialize Option<Vec<Arc<T>>>
pub fn serialize_option_arc_vec<S, T>(
    opt: &Option<Vec<Arc<T>>>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    T: Serialize,
{
    match opt {
        Some(vec) => {
            use serde::ser::SerializeSeq;
            let mut seq = serializer.serialize_seq(Some(vec.len()))?;
            for item in vec {
                seq.serialize_element(item.as_ref())?;
            }
            seq.end()
        }
        None => serializer.serialize_none(),
    }
}

/// Deserialize Option<Vec<Arc<T>>>
pub fn deserialize_option_arc_vec<'de, D, T>(
    deserializer: D,
) -> Result<Option<Vec<Arc<T>>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    let opt: Option<Vec<T>> = Option::deserialize(deserializer)?;
    Ok(opt.map(|vec| vec.into_iter().map(Arc::new).collect()))
}

/// Serialize Vec<(Arc<T>, Arc<T>)> for pattern-matching cases
pub fn serialize_arc_tuple_vec<S, T>(
    vec: &Vec<(Arc<T>, Arc<T>)>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    T: Serialize,
{
    use serde::ser::SerializeSeq;
    let mut seq = serializer.serialize_seq(Some(vec.len()))?;
    for (a, b) in vec {
        seq.serialize_element(&(a.as_ref(), b.as_ref()))?;
    }
    seq.end()
}

/// Deserialize Vec<(Arc<T>, Arc<T>)> for pattern-matching cases
pub fn deserialize_arc_tuple_vec<'de, D, T>(
    deserializer: D,
) -> Result<Vec<(Arc<T>, Arc<T>)>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    let vec: Vec<(T, T)> = Vec::deserialize(deserializer)?;
    Ok(vec
        .into_iter()
        .map(|(a, b)| (Arc::new(a), Arc::new(b)))
        .collect())
}

/// Serialize rpds::Vector<Arc<T>> by serializing each inner value
pub fn serialize_rpds_arc_vec<S, T>(
    vec: &rpds::Vector<Arc<T>, archery::ArcK>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    T: Serialize,
{
    use serde::ser::SerializeSeq;
    let mut seq = serializer.serialize_seq(Some(vec.len()))?;
    for item in vec.iter() {
        seq.serialize_element(item.as_ref())?;
    }
    seq.end()
}

/// Deserialize rpds::Vector<Arc<T>> by deserializing each value and wrapping in Arc
pub fn deserialize_rpds_arc_vec<'de, D, T>(
    deserializer: D,
) -> Result<rpds::Vector<Arc<T>, archery::ArcK>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    let vec: Vec<T> = Vec::deserialize(deserializer)?;
    Ok(vec.into_iter().map(Arc::new).collect())
}

/// Serialize Option<rpds::Vector<Arc<T>>>
pub fn serialize_option_rpds_arc_vec<S, T>(
    opt: &Option<rpds::Vector<Arc<T>, archery::ArcK>>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    T: Serialize,
{
    match opt {
        Some(vec) => {
            use serde::ser::SerializeSeq;
            let mut seq = serializer.serialize_seq(Some(vec.len()))?;
            for item in vec.iter() {
                seq.serialize_element(item.as_ref())?;
            }
            seq.end()
        }
        None => serializer.serialize_none(),
    }
}

/// Deserialize Option<rpds::Vector<Arc<T>>>
pub fn deserialize_option_rpds_arc_vec<'de, D, T>(
    deserializer: D,
) -> Result<Option<rpds::Vector<Arc<T>, archery::ArcK>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    let opt: Option<Vec<T>> = Option::deserialize(deserializer)?;
    Ok(opt.map(|vec| vec.into_iter().map(Arc::new).collect()))
}

/// Serialize rpds::Vector<(Arc<T>, Arc<T>)> for tuple pairs
pub fn serialize_rpds_arc_tuple_vec<S, T>(
    vec: &rpds::Vector<(Arc<T>, Arc<T>), archery::ArcK>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    T: Serialize,
{
    use serde::ser::SerializeSeq;
    let mut seq = serializer.serialize_seq(Some(vec.len()))?;
    for (a, b) in vec.iter() {
        seq.serialize_element(&(a.as_ref(), b.as_ref()))?;
    }
    seq.end()
}

/// Deserialize rpds::Vector<(Arc<T>, Arc<T>)> for tuple pairs
pub fn deserialize_rpds_arc_tuple_vec<'de, D, T>(
    deserializer: D,
) -> Result<rpds::Vector<(Arc<T>, Arc<T>), archery::ArcK>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    let vec: Vec<(T, T)> = Vec::deserialize(deserializer)?;
    Ok(vec
        .into_iter()
        .map(|(a, b)| (Arc::new(a), Arc::new(b)))
        .collect())
}

/// Serialize rpds::Vector<rpds::Vector<Arc<T>>> for nested vectors
pub fn serialize_rpds_nested_arc_vec<S, T>(
    vec: &rpds::Vector<rpds::Vector<Arc<T>, archery::ArcK>, archery::ArcK>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    T: Serialize,
{
    use serde::ser::SerializeSeq;
    let mut seq = serializer.serialize_seq(Some(vec.len()))?;
    for inner_vec in vec.iter() {
        let inner: Vec<&T> = inner_vec.iter().map(|item| item.as_ref()).collect();
        seq.serialize_element(&inner)?;
    }
    seq.end()
}

/// Deserialize rpds::Vector<rpds::Vector<Arc<T>>> for nested vectors
pub fn deserialize_rpds_nested_arc_vec<'de, D, T>(
    deserializer: D,
) -> Result<rpds::Vector<rpds::Vector<Arc<T>, archery::ArcK>, archery::ArcK>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    let vec: Vec<Vec<T>> = Vec::deserialize(deserializer)?;
    Ok(vec
        .into_iter()
        .map(|inner| inner.into_iter().map(Arc::new).collect())
        .collect())
}

/// Serialize rpds::Vector<(rpds::Vector<Arc<T>>, Arc<T>)> for branch vectors
/// Used by RholangBranchVector = Vector<(RholangNodeVector, Arc<RholangNode>), ArcK>
pub fn serialize_rpds_branch_vec<S, T>(
    vec: &rpds::Vector<(rpds::Vector<Arc<T>, archery::ArcK>, Arc<T>), archery::ArcK>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    T: Serialize,
{
    use serde::ser::SerializeSeq;
    let mut seq = serializer.serialize_seq(Some(vec.len()))?;
    for (inner_vec, arc_node) in vec.iter() {
        let inner: Vec<&T> = inner_vec.iter().map(|item| item.as_ref()).collect();
        seq.serialize_element(&(inner, arc_node.as_ref()))?;
    }
    seq.end()
}

/// Deserialize rpds::Vector<(rpds::Vector<Arc<T>>, Arc<T>)> for branch vectors
pub fn deserialize_rpds_branch_vec<'de, D, T>(
    deserializer: D,
) -> Result<rpds::Vector<(rpds::Vector<Arc<T>, archery::ArcK>, Arc<T>), archery::ArcK>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    let vec: Vec<(Vec<T>, T)> = Vec::deserialize(deserializer)?;
    Ok(vec
        .into_iter()
        .map(|(inner_vec, node)| {
            let rpds_vec: rpds::Vector<Arc<T>, archery::ArcK> =
                inner_vec.into_iter().map(Arc::new).collect();
            (rpds_vec, Arc::new(node))
        })
        .collect())
}
