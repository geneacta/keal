//! What a Keal value is, in bytes.
//!
//! Everything the language runs on today is a Rust `Value` — a tagged enum
//! whose size and lifetime Rust decides. Native code cannot borrow that: it
//! needs to know that a `Point` is sixteen bytes with `y` at offset eight,
//! and who frees it. This module is where those answers are written down.
//!
//! Three decisions shape the rest:
//!
//! * **Fields keep declaration order.** Reordering them would save padding,
//!   but a struct whose layout the author cannot predict is a struct C cannot
//!   be handed. Predictability and interop beat a few bytes.
//! * **Every heap object begins with its reference count.** That is the memory
//!   model the language chose; putting the count in the object rather than
//!   beside it keeps a value one pointer wide.
//! * **A nullable uses a spare bit pattern where one exists.** A pointer has
//!   null, a `Bool` has every value but 0 and 1. Only when there is no spare
//!   pattern does `T?` cost a separate tag — which is why `Int?` is twice the
//!   size of `Int` and `String?` is the same size as `String`.

use std::fmt;

use crate::types::Type;

/// A size and an alignment, both in bytes.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Layout {
    pub size: u64,
    pub align: u64,
}

impl Layout {
    pub const fn new(size: u64, align: u64) -> Layout {
        Layout { size, align }
    }

    /// The next offset at or after `at` that this layout may start on.
    pub fn align_to(align: u64, at: u64) -> u64 {
        debug_assert!(align.is_power_of_two());
        (at + align - 1) & !(align - 1)
    }
}

/// The width of a pointer and of the reference count that precedes every
/// heap object. Both are the target's word; 64-bit for now.
pub const WORD: u64 = 8;

/// How a value is represented in memory.
#[derive(Clone, PartialEq, Debug)]
pub enum Repr {
    /// Occupies nothing. `Unit`, and `Nothing`, which has no values at all.
    Zero,
    /// A signed 64-bit integer.
    Int,
    /// An IEEE 754 double.
    Float,
    /// One byte, holding 0 or 1.
    Bool,
    /// Two integers, inline: a range is a value, not an object.
    Range,
    /// One pointer to a reference-counted object.
    Ref(RefKind),
    /// A value that may be absent. `niche` records whether it fits in a spare
    /// bit pattern of the type underneath.
    Nullable(Box<Repr>),
    /// A value whose type is not known statically: a tag and a payload.
    Any,
    /// A type parameter, whose representation is fixed at each instantiation.
    /// A monomorphising backend never sees one; it is reported, not laid out.
    Generic,
}

/// What a pointer points at. Each has its own header after the count.
#[derive(Clone, PartialEq, Debug)]
pub enum RefKind {
    /// `{ rc, len, bytes… }` — immutable, so it may be shared freely.
    Str,
    /// `{ rc, len, capacity, data }`.
    List,
    /// `{ rc, len, capacity, entries, order }`.
    Map,
    /// `{ rc, fields… }`, the fields in declaration order.
    Instance(String),
    /// `{ rc, code, captured… }`.
    Function,
}

impl Repr {
    /// How a checked type is represented.
    pub fn of(ty: &Type) -> Repr {
        match ty {
            Type::Unit | Type::Never => Repr::Zero,
            Type::Int => Repr::Int,
            Type::Float => Repr::Float,
            Type::Bool => Repr::Bool,
            Type::Range => Repr::Range,
            Type::Str => Repr::Ref(RefKind::Str),
            Type::List(_) => Repr::Ref(RefKind::List),
            Type::Map(_, _) => Repr::Ref(RefKind::Map),
            Type::Fun(_) => Repr::Ref(RefKind::Function),
            Type::Class(name, _) => Repr::Ref(RefKind::Instance(name.to_string())),
            Type::Nullable(inner) => Repr::Nullable(Box::new(Repr::of(inner))),
            // `null` on its own is only ever assigned into a `T?`.
            Type::Null => Repr::Nullable(Box::new(Repr::Zero)),
            Type::Any => Repr::Any,
            Type::Param(_) | Type::SelfTy => Repr::Generic,
            Type::Error => Repr::Generic,
        }
    }

    /// The size and alignment this representation occupies where it is stored
    /// — inline in a struct, in a list, or in a register.
    ///
    /// Returns `None` for a type parameter, which has no layout until the
    /// backend instantiates it.
    pub fn layout(&self) -> Option<Layout> {
        Some(match self {
            Repr::Zero => Layout::new(0, 1),
            Repr::Int | Repr::Float => Layout::new(8, 8),
            Repr::Bool => Layout::new(1, 1),
            Repr::Range => Layout::new(16, 8),
            Repr::Ref(_) => Layout::new(WORD, WORD),
            // A tag and a payload, both a word wide. A value too wide for the
            // payload is boxed on the way in, which is what makes `Any` one
            // size whatever it holds.
            Repr::Any => Layout::new(2 * WORD, WORD),
            Repr::Nullable(inner) => {
                let inner_layout = inner.layout()?;
                if self.has_niche() {
                    inner_layout
                } else {
                    // No spare pattern: a tag beside the value, padded to the
                    // value's alignment.
                    let align = inner_layout.align.max(1);
                    let size = Layout::align_to(align, align + inner_layout.size);
                    Layout::new(size, align)
                }
            }
            Repr::Generic => return None,
        })
    }

    /// True when `T?` costs nothing over `T`, because `T` leaves a bit
    /// pattern spare that can stand for null.
    pub fn has_niche(&self) -> bool {
        match self {
            Repr::Nullable(inner) => inner.has_niche(),
            // A pointer is never null while it holds an object.
            Repr::Ref(_) => true,
            // A byte holding 0 or 1 has 254 patterns to spare.
            Repr::Bool => true,
            // `Any` already has a tag, so one more value for it costs nothing.
            Repr::Any => true,
            // Nothing has no values, so an optional one needs only the tag,
            // which is the value itself.
            Repr::Zero => true,
            _ => false,
        }
    }

    /// True when a value of this representation owns a reference that must be
    /// released when it goes away. The backend emits a decrement for these
    /// and nothing for the rest.
    pub fn is_counted(&self) -> bool {
        match self {
            Repr::Ref(_) => true,
            Repr::Nullable(inner) => inner.is_counted(),
            // Whether it counts depends on the tag, so the check is dynamic.
            Repr::Any => true,
            _ => false,
        }
    }

    /// True when a value of this representation can cross into C with no
    /// question of ownership.
    ///
    /// A pointer to a counted object has the *shape* C expects, but not the
    /// lifetime: something has to release it, and C does not know how. Those
    /// are excluded here and will need an explicit borrow or transfer at the
    /// boundary rather than a silent pass.
    pub fn is_c_compatible(&self) -> bool {
        match self {
            Repr::Int | Repr::Float | Repr::Bool | Repr::Zero | Repr::Range => true,
            Repr::Ref(_) => false,
            Repr::Nullable(inner) => inner.is_c_compatible(),
            Repr::Any | Repr::Generic => false,
        }
    }
}

impl fmt::Display for Repr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Repr::Zero => write!(f, "zero-sized"),
            Repr::Int => write!(f, "i64"),
            Repr::Float => write!(f, "f64"),
            Repr::Bool => write!(f, "i8"),
            Repr::Range => write!(f, "{{i64, i64}}"),
            Repr::Ref(k) => write!(f, "*{}", k),
            Repr::Nullable(inner) => {
                if self.has_niche() {
                    write!(f, "{}?", inner)
                } else {
                    write!(f, "{{i8, {}}}", inner)
                }
            }
            Repr::Any => write!(f, "{{tag, word}}"),
            Repr::Generic => write!(f, "<per instantiation>"),
        }
    }
}

impl fmt::Display for RefKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RefKind::Str => write!(f, "String"),
            RefKind::List => write!(f, "List"),
            RefKind::Map => write!(f, "Map"),
            RefKind::Function => write!(f, "Function"),
            RefKind::Instance(name) => write!(f, "{}", name),
        }
    }
}

/// One field of a laid-out object.
pub struct FieldLayout {
    pub name: String,
    pub ty: Type,
    pub repr: Repr,
    /// Measured from the start of the object, so it includes the header.
    pub offset: u64,
    pub size: u64,
    /// Bytes of padding inserted before this field to satisfy its alignment.
    pub padding_before: u64,
}

/// A class or record, laid out.
pub struct ObjectLayout {
    pub name: String,
    pub fields: Vec<FieldLayout>,
    /// The whole object, reference count included.
    pub size: u64,
    pub align: u64,
    /// True when the class takes type parameters, so this is one shape among
    /// however many the program instantiates.
    pub generic: bool,
}

impl ObjectLayout {
    pub fn padding(&self) -> u64 {
        self.fields.iter().map(|f| f.padding_before).sum::<u64>() + self.tail_padding()
    }

    fn tail_padding(&self) -> u64 {
        let used = self
            .fields
            .last()
            .map(|f| f.offset + f.size)
            .unwrap_or(WORD);
        self.size - used
    }
}

/// Lays out an object: the reference count first, then the fields in the
/// order they were declared, each at the next offset its alignment allows.
pub fn object_layout(name: &str, fields: &[(String, Type)], generic: bool) -> ObjectLayout {
    // Every heap object starts with its count, so a value is one pointer.
    let mut offset = WORD;
    let mut align = WORD;
    let mut out = Vec::with_capacity(fields.len());

    for (fname, ty) in fields {
        let repr = Repr::of(ty);
        let layout = repr.layout().unwrap_or(Layout::new(WORD, WORD));
        let at = Layout::align_to(layout.align.max(1), offset);
        out.push(FieldLayout {
            name: fname.clone(),
            ty: ty.clone(),
            repr,
            offset: at,
            size: layout.size,
            padding_before: at - offset,
        });
        offset = at + layout.size;
        align = align.max(layout.align.max(1));
    }

    ObjectLayout {
        name: name.to_string(),
        fields: out,
        size: Layout::align_to(align, offset),
        align,
        generic,
    }
}

/// The representations that do not depend on any declaration.
pub fn builtin_reprs() -> Vec<(&'static str, Repr)> {
    vec![
        ("Int", Repr::Int),
        ("Float", Repr::Float),
        ("Bool", Repr::Bool),
        ("Unit", Repr::Zero),
        ("Range", Repr::Range),
        ("String", Repr::Ref(RefKind::Str)),
        ("List<T>", Repr::Ref(RefKind::List)),
        ("Map<K, V>", Repr::Ref(RefKind::Map)),
        ("(A) -> B", Repr::Ref(RefKind::Function)),
        ("Any", Repr::Any),
        ("Int?", Repr::Nullable(Box::new(Repr::Int))),
        ("Bool?", Repr::Nullable(Box::new(Repr::Bool))),
        ("String?", Repr::Nullable(Box::new(Repr::Ref(RefKind::Str)))),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_nullable_pointer_costs_nothing_extra() {
        let s = Repr::Ref(RefKind::Str);
        let opt = Repr::Nullable(Box::new(s.clone()));
        assert_eq!(s.layout(), opt.layout());
    }

    #[test]
    fn a_nullable_int_needs_a_tag() {
        let n = Repr::Int.layout().unwrap();
        let opt = Repr::Nullable(Box::new(Repr::Int)).layout().unwrap();
        assert_eq!(n.size, 8);
        assert_eq!(opt.size, 16);
    }

    #[test]
    fn a_nullable_bool_fits_in_its_byte() {
        let opt = Repr::Nullable(Box::new(Repr::Bool)).layout().unwrap();
        assert_eq!(opt, Layout::new(1, 1));
    }

    #[test]
    fn fields_keep_declaration_order_and_pay_for_it() {
        // A byte between two words costs seven bytes of padding, which
        // reordering would recover — and which predictability buys.
        let fields = vec![
            ("flag".to_string(), Type::Bool),
            ("count".to_string(), Type::Int),
        ];
        let laid = object_layout("Mixed", &fields, false);
        assert_eq!(laid.fields[0].offset, 8, "after the reference count");
        assert_eq!(laid.fields[1].offset, 16, "the integer is realigned");
        assert_eq!(laid.size, 24);
        assert_eq!(laid.padding(), 7);
    }

    #[test]
    fn an_empty_record_is_just_its_count() {
        let laid = object_layout("Empty", &[], false);
        assert_eq!(laid.size, WORD);
    }
}
