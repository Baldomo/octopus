//! The shape of every op's args: the server checks requests against it and the CLI builds its flags from it.

use serde_json::Value;

use crate::frames::Error;

#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    Text,
    Boolean,
    Integer,
    /// One of these wire spellings.
    Choice(Vec<String>),
    /// A workspace id on the wire; the CLI takes the workspace's name.
    Workspace,
    /// The value or null, which the CLI spells `none`; `none` says what null means.
    Nullable {
        value: Box<Shape>,
        none: &'static str,
    },
    /// Text, or `word` for `means`; the wire carries either as text.
    Keyword {
        word: &'static str,
        means: &'static str,
    },
    List(Box<Shape>),
    Record(Vec<Field>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub name: &'static str,
    pub doc: &'static str,
    pub shape: Shape,
}

pub trait Arg {
    fn shape() -> Shape;
}

/// A struct whose fields are args, declared with `record!`.
pub trait Fields {
    fn fields() -> Vec<Field>;
}

#[derive(Default)]
struct Violations {
    missing: Vec<String>,
    unknown: Vec<String>,
}

fn at(path: &str, field: &str) -> String {
    if path.is_empty() { field.to_string() } else { format!("{path}.{field}") }
}

impl Shape {
    /// The word the CLI takes for a value spelled by no text of its own, and what it stands for.
    pub fn keyword(&self) -> Option<(&'static str, &'static str)> {
        match self {
            Self::Nullable { none, .. } => Some(("none", none)),
            Self::Keyword { word, means } => Some((word, means)),
            _ => None,
        }
    }

    /// Type mismatches are left to serde, which names the first one.
    fn check(&self, value: &Value, path: &str, found: &mut Violations) {
        match (self, value) {
            (Self::Record(fields), Value::Object(object)) => {
                for field in fields {
                    match object.get(field.name) {
                        Some(value) => field.shape.check(value, &at(path, field.name), found),
                        None => found.missing.push(at(path, field.name)),
                    }
                }
                let undeclared = object.keys().filter(|key| fields.iter().all(|field| field.name != key.as_str()));
                found.unknown.extend(undeclared.map(|key| at(path, key)));
            }
            (Self::Nullable { value: shape, .. }, value) if !value.is_null() => shape.check(value, path, found),
            (Self::List(item), Value::Array(items)) => {
                for (index, value) in items.iter().enumerate() {
                    item.check(value, &format!("{path}[{index}]"), found);
                }
            }
            _ => {}
        }
    }
}

/// Every field absent at any depth, then every undeclared one, each named by its dotted path.
pub fn check(fields: Vec<Field>, args: &Value) -> Result<(), Error> {
    if !args.is_object() {
        return Err(Error::invalid("args must be an object"));
    }
    let mut found = Violations::default();
    Shape::Record(fields).check(args, "", &mut found);
    if !found.missing.is_empty() {
        return Err(Error::MissingField { fields: found.missing });
    }
    if !found.unknown.is_empty() {
        return Err(Error::UnknownField { fields: found.unknown });
    }
    Ok(())
}

/// Gives each type the arg shape `$shape`, an expression that may name `Self`.
macro_rules! arg_shape {
    ($shape:expr => $($ty:ty),+ $(,)?) => {$(
        impl $crate::schema::Arg for $ty {
            fn shape() -> $crate::schema::Shape {
                $shape
            }
        }
    )+};
}
pub(crate) use arg_shape;

/// Spells each type on the wire as its `Display` text, read back through its `FromStr`.
macro_rules! spelled {
    ($($ty:ty),+ $(,)?) => {$(
        impl serde::Serialize for $ty {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_str(self)
            }
        }

        impl<'de> serde::Deserialize<'de> for $ty {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                <String as serde::Deserialize>::deserialize(deserializer)?.parse().map_err(serde::de::Error::custom)
            }
        }
    )+};
}
pub(crate) use spelled;

arg_shape!(Shape::Text => String);
arg_shape!(Shape::Boolean => bool);
arg_shape!(Shape::Integer => u16, u64);

impl<T: Arg> Arg for Vec<T> {
    fn shape() -> Shape {
        Shape::List(Box::new(T::shape()))
    }
}

/// Declares a struct of args: every field required, none undeclared, each documented for the CLI and its skill.
macro_rules! record {
    ($(#[doc = $doc:literal])* $name:ident { $($(#[doc = $field_doc:literal])* $field:ident : $ty:ty),* $(,)? }) => {
        $(#[doc = $doc])*
        #[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        pub struct $name { $($(#[doc = $field_doc])* pub $field: $ty),* }

        impl $crate::schema::Fields for $name {
            fn fields() -> Vec<$crate::schema::Field> {
                vec![$($crate::schema::Field {
                    name: stringify!($field),
                    doc: concat!($($field_doc),*).trim(),
                    shape: <$ty as $crate::schema::Arg>::shape(),
                }),*]
            }
        }

        impl $crate::schema::Arg for $name {
            fn shape() -> $crate::schema::Shape {
                $crate::schema::Shape::Record(<Self as $crate::schema::Fields>::fields())
            }
        }
    };
}
pub(crate) use record;
