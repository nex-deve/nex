//! The static types of NEX.

use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Type {
    Int,
    Float,
    Bool,
    Str,
    /// The type of functions that return no value. It has no literal and
    /// cannot be stored in variables.
    Unit,
    /// `[T]`: an ordered, growable list of `T`.
    Array(Box<Type>),
    /// `{K: V}`: a map from keys to values, iterated in sorted key order.
    Map(Box<Type>, Box<Type>),
    /// Any JSON value (object, array, string, number, bool or null).
    Json,
}

impl Type {
    pub fn from_name(name: &str) -> Option<Type> {
        match name {
            "Int" => Some(Type::Int),
            "Float" => Some(Type::Float),
            "Bool" => Some(Type::Bool),
            "Str" => Some(Type::Str),
            "Unit" => Some(Type::Unit),
            "Json" => Some(Type::Json),
            _ => None,
        }
    }

    pub fn array(elem: Type) -> Type {
        Type::Array(Box::new(elem))
    }

    pub fn map(key: Type, value: Type) -> Type {
        Type::Map(Box::new(key), Box::new(value))
    }

    /// Types that may be used as map keys.
    pub fn is_key(&self) -> bool {
        matches!(self, Type::Int | Type::Bool | Type::Str)
    }

    /// Types with a total order usable by `<` and `sorted()`.
    pub fn is_ordered(&self) -> bool {
        matches!(self, Type::Int | Type::Float | Type::Str)
    }

    /// True if `Unit` appears anywhere inside this type.
    pub fn contains_unit(&self) -> bool {
        match self {
            Type::Unit => true,
            Type::Array(e) => e.contains_unit(),
            Type::Map(k, v) => k.contains_unit() || v.contains_unit(),
            _ => false,
        }
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::Int => f.write_str("Int"),
            Type::Float => f.write_str("Float"),
            Type::Bool => f.write_str("Bool"),
            Type::Str => f.write_str("Str"),
            Type::Unit => f.write_str("Unit"),
            Type::Json => f.write_str("Json"),
            Type::Array(e) => write!(f, "[{e}]"),
            Type::Map(k, v) => write!(f, "{{{k}: {v}}}"),
        }
    }
}
