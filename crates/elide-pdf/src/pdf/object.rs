//! The objects a document is written with: values owned and built in
//! memory, then serialized through pdf-writer by a [`Document`].
//!
//! [`Document`]: super::writer::Document

use std::collections::BTreeMap;

/// The number of an object in a document being written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct Id(pub(crate) u32);

/// A value written into a document.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Real(f64),
    Name(Vec<u8>),
    String(Vec<u8>),
    Array(Vec<Value>),
    Dict(Dictionary),
    Stream(Box<Stream>),
    Ref(Id),
}

impl Value {
    /// The name `name`.
    pub(crate) fn name(name: impl AsRef<[u8]>) -> Self {
        Self::Name(name.as_ref().to_vec())
    }

    /// The string of bytes `bytes`.
    pub(crate) fn string(bytes: impl AsRef<[u8]>) -> Self {
        Self::String(bytes.as_ref().to_vec())
    }

    /// The number `value`: an integer when it is a whole one, as a reader
    /// reads it back, else a real.
    pub(crate) fn number(value: f64) -> Self {
        if value.fract() == 0.0 && value.abs() < (1u64 << 53) as f64 {
            Self::Int(value as i64)
        } else {
            Self::Real(value)
        }
    }

    /// The value as a number, integer or real.
    pub(crate) fn as_number(&self) -> Option<f64> {
        match *self {
            Self::Int(i) => Some(i as f64),
            Self::Real(r) => Some(r),
            _ => None,
        }
    }

    /// The value as an integer.
    pub(crate) fn as_int(&self) -> Option<i64> {
        match *self {
            Self::Int(i) => Some(i),
            _ => None,
        }
    }

    /// The value as a name.
    pub(crate) fn as_name(&self) -> Option<&[u8]> {
        match self {
            Self::Name(name) => Some(name),
            _ => None,
        }
    }

    /// The value as a string.
    pub(crate) fn as_string(&self) -> Option<&[u8]> {
        match self {
            Self::String(bytes) => Some(bytes),
            _ => None,
        }
    }

    /// The value as a dictionary: its own, or a stream's.
    pub(crate) fn as_dict(&self) -> Option<&Dictionary> {
        match self {
            Self::Dict(dict) => Some(dict),
            Self::Stream(stream) => Some(&stream.dict),
            _ => None,
        }
    }

    /// The value as a dictionary, mutably: its own, or a stream's.
    pub(crate) fn as_dict_mut(&mut self) -> Option<&mut Dictionary> {
        match self {
            Self::Dict(dict) => Some(dict),
            Self::Stream(stream) => Some(&mut stream.dict),
            _ => None,
        }
    }

    /// The value as a reference.
    pub(crate) fn as_ref_id(&self) -> Option<Id> {
        match *self {
            Self::Ref(id) => Some(id),
            _ => None,
        }
    }
}

impl From<bool> for Value {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<i64> for Value {
    fn from(value: i64) -> Self {
        Self::Int(value)
    }
}

impl From<i32> for Value {
    fn from(value: i32) -> Self {
        Self::Int(value.into())
    }
}

impl From<u32> for Value {
    fn from(value: u32) -> Self {
        Self::Int(value.into())
    }
}

impl From<usize> for Value {
    fn from(value: usize) -> Self {
        Self::Int(i64::try_from(value).unwrap_or(i64::MAX))
    }
}

impl From<f64> for Value {
    fn from(value: f64) -> Self {
        Self::Real(value)
    }
}

/// A string is written as a name, as a dictionary key or type is.
impl From<&str> for Value {
    fn from(value: &str) -> Self {
        Self::name(value)
    }
}

impl From<Id> for Value {
    fn from(id: Id) -> Self {
        Self::Ref(id)
    }
}

impl From<Vec<Value>> for Value {
    fn from(items: Vec<Value>) -> Self {
        Self::Array(items)
    }
}

impl From<Dictionary> for Value {
    fn from(dict: Dictionary) -> Self {
        Self::Dict(dict)
    }
}

impl From<Stream> for Value {
    fn from(stream: Stream) -> Self {
        Self::Stream(Box::new(stream))
    }
}

/// A dictionary written into a document, its keys in order.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Dictionary(BTreeMap<Vec<u8>, Value>);

impl Dictionary {
    /// An empty dictionary.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// The value of `key`.
    pub(crate) fn get(&self, key: &[u8]) -> Option<&Value> {
        self.0.get(key)
    }

    /// The value of `key`, mutably.
    pub(crate) fn get_mut(&mut self, key: &[u8]) -> Option<&mut Value> {
        self.0.get_mut(key)
    }

    /// Whether the dictionary holds `key`.
    pub(crate) fn has(&self, key: &[u8]) -> bool {
        self.0.contains_key(key)
    }

    /// Set `key` to `value`.
    pub(crate) fn set(&mut self, key: impl AsRef<[u8]>, value: impl Into<Value>) {
        self.0.insert(key.as_ref().to_vec(), value.into());
    }

    /// Remove `key`, returning its value.
    pub(crate) fn remove(&mut self, key: &[u8]) -> Option<Value> {
        self.0.remove(key)
    }

    /// Each key and its value, in key order.
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&[u8], &Value)> {
        self.0.iter().map(|(key, value)| (key.as_slice(), value))
    }

    /// Each key and its value, mutably.
    pub(crate) fn iter_mut(&mut self) -> impl Iterator<Item = (&[u8], &mut Value)> {
        self.0
            .iter_mut()
            .map(|(key, value)| (key.as_slice(), value))
    }

    /// Keep only the entries `keep` accepts.
    pub(crate) fn retain(&mut self, mut keep: impl FnMut(&[u8], &Value) -> bool) {
        self.0.retain(|key, value| keep(key, value));
    }
}

impl<K: AsRef<[u8]>, V: Into<Value>> FromIterator<(K, V)> for Dictionary {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        Self(
            iter.into_iter()
                .map(|(key, value)| (key.as_ref().to_vec(), value.into()))
                .collect(),
        )
    }
}

/// A stream written into a document: its dictionary (without `/Length`,
/// which is written from its data) and its data as stored.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Stream {
    pub(crate) dict: Dictionary,
    pub(crate) data: Vec<u8>,
}

impl Stream {
    /// A stream of `dict` holding `data` as stored.
    pub(crate) fn new(dict: Dictionary, data: Vec<u8>) -> Self {
        Self { dict, data }
    }
}

/// A [`Dictionary`] of the pairs given, each key a string and each value
/// anything a [`Value`] is made from.
macro_rules! dict {
    ($($key:expr => $value:expr),* $(,)?) => {{
        #[allow(unused_mut)]
        let mut dict = $crate::pdf::object::Dictionary::new();
        $(dict.set($key, $value);)*
        dict
    }};
}

pub(crate) use dict;
