//! Reject ignored fields throughout reused core serde types without changing
//! their existing protocol formats. Every nested deserializer stays wrapped.
use serde::de::{self, DeserializeSeed, EnumAccess, MapAccess, SeqAccess, VariantAccess, Visitor};
use std::fmt;

pub(super) struct Strict<D>(pub(super) D);
struct StrictVisitor<V>(V);
struct StrictSeed<S>(S);
struct StrictSeq<A>(A);
struct StrictMap<A>(A);
struct StrictEnum<A>(A);
struct StrictVariant<A>(A);

macro_rules! deserialize_value {
    ($method:ident) => {
        fn $method<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
            self.0.$method(StrictVisitor(visitor))
        }
    };
}
impl<'de, D: de::Deserializer<'de>> de::Deserializer<'de> for Strict<D> {
    type Error = D::Error;
    deserialize_value!(deserialize_any);
    deserialize_value!(deserialize_bool);
    deserialize_value!(deserialize_i8);
    deserialize_value!(deserialize_i16);
    deserialize_value!(deserialize_i32);
    deserialize_value!(deserialize_i64);
    deserialize_value!(deserialize_i128);
    deserialize_value!(deserialize_u8);
    deserialize_value!(deserialize_u16);
    deserialize_value!(deserialize_u32);
    deserialize_value!(deserialize_u64);
    deserialize_value!(deserialize_u128);
    deserialize_value!(deserialize_f32);
    deserialize_value!(deserialize_f64);
    deserialize_value!(deserialize_char);
    deserialize_value!(deserialize_str);
    deserialize_value!(deserialize_string);
    deserialize_value!(deserialize_bytes);
    deserialize_value!(deserialize_byte_buf);
    deserialize_value!(deserialize_option);
    deserialize_value!(deserialize_unit);
    deserialize_value!(deserialize_seq);
    deserialize_value!(deserialize_map);
    deserialize_value!(deserialize_identifier);

    fn deserialize_unit_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.0.deserialize_unit_struct(name, StrictVisitor(visitor))
    }
    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.0
            .deserialize_newtype_struct(name, StrictVisitor(visitor))
    }
    fn deserialize_tuple<V: Visitor<'de>>(
        self,
        len: usize,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.0.deserialize_tuple(len, StrictVisitor(visitor))
    }
    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        len: usize,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.0
            .deserialize_tuple_struct(name, len, StrictVisitor(visitor))
    }
    fn deserialize_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.0
            .deserialize_struct(name, fields, StrictVisitor(visitor))
    }
    fn deserialize_enum<V: Visitor<'de>>(
        self,
        name: &'static str,
        variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.0
            .deserialize_enum(name, variants, StrictVisitor(visitor))
    }
    fn deserialize_ignored_any<V: Visitor<'de>>(
        self,
        _visitor: V,
    ) -> Result<V::Value, Self::Error> {
        Err(de::Error::custom("unknown checkpoint field"))
    }
    fn is_human_readable(&self) -> bool {
        self.0.is_human_readable()
    }
}

macro_rules! visit_value {
    ($method:ident, $ty:ty) => {
        fn $method<E: de::Error>(self, value: $ty) -> Result<Self::Value, E> {
            self.0.$method(value)
        }
    };
}
impl<'de, V: Visitor<'de>> Visitor<'de> for StrictVisitor<V> {
    type Value = V::Value;
    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.expecting(formatter)
    }
    visit_value!(visit_bool, bool);
    visit_value!(visit_i8, i8);
    visit_value!(visit_i16, i16);
    visit_value!(visit_i32, i32);
    visit_value!(visit_i64, i64);
    visit_value!(visit_i128, i128);
    visit_value!(visit_u8, u8);
    visit_value!(visit_u16, u16);
    visit_value!(visit_u32, u32);
    visit_value!(visit_u64, u64);
    visit_value!(visit_u128, u128);
    visit_value!(visit_f32, f32);
    visit_value!(visit_f64, f64);
    visit_value!(visit_char, char);
    visit_value!(visit_str, &str);
    visit_value!(visit_borrowed_str, &'de str);
    visit_value!(visit_string, String);
    visit_value!(visit_bytes, &[u8]);
    visit_value!(visit_borrowed_bytes, &'de [u8]);
    visit_value!(visit_byte_buf, Vec<u8>);
    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        self.0.visit_unit()
    }
    fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
        self.0.visit_none()
    }
    fn visit_some<D: de::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> Result<Self::Value, D::Error> {
        self.0.visit_some(Strict(deserializer))
    }
    fn visit_newtype_struct<D: de::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> Result<Self::Value, D::Error> {
        self.0.visit_newtype_struct(Strict(deserializer))
    }
    fn visit_seq<A: SeqAccess<'de>>(self, access: A) -> Result<Self::Value, A::Error> {
        self.0.visit_seq(StrictSeq(access))
    }
    fn visit_map<A: MapAccess<'de>>(self, access: A) -> Result<Self::Value, A::Error> {
        self.0.visit_map(StrictMap(access))
    }
    fn visit_enum<A: EnumAccess<'de>>(self, access: A) -> Result<Self::Value, A::Error> {
        self.0.visit_enum(StrictEnum(access))
    }
}
impl<'de, S: DeserializeSeed<'de>> DeserializeSeed<'de> for StrictSeed<S> {
    type Value = S::Value;
    fn deserialize<D: de::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> Result<Self::Value, D::Error> {
        self.0.deserialize(Strict(deserializer))
    }
}
impl<'de, A: SeqAccess<'de>> SeqAccess<'de> for StrictSeq<A> {
    type Error = A::Error;
    fn next_element_seed<T: DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, Self::Error> {
        self.0.next_element_seed(StrictSeed(seed))
    }
    fn size_hint(&self) -> Option<usize> {
        self.0.size_hint()
    }
}
impl<'de, A: MapAccess<'de>> MapAccess<'de> for StrictMap<A> {
    type Error = A::Error;
    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, Self::Error> {
        self.0.next_key_seed(StrictSeed(seed))
    }
    fn next_value_seed<V: DeserializeSeed<'de>>(
        &mut self,
        seed: V,
    ) -> Result<V::Value, Self::Error> {
        self.0.next_value_seed(StrictSeed(seed))
    }
    fn size_hint(&self) -> Option<usize> {
        self.0.size_hint()
    }
}
impl<'de, A: EnumAccess<'de>> EnumAccess<'de> for StrictEnum<A> {
    type Error = A::Error;
    type Variant = StrictVariant<A::Variant>;
    fn variant_seed<V: DeserializeSeed<'de>>(
        self,
        seed: V,
    ) -> Result<(V::Value, Self::Variant), Self::Error> {
        self.0
            .variant_seed(StrictSeed(seed))
            .map(|(value, variant)| (value, StrictVariant(variant)))
    }
}
impl<'de, A: VariantAccess<'de>> VariantAccess<'de> for StrictVariant<A> {
    type Error = A::Error;
    fn unit_variant(self) -> Result<(), Self::Error> {
        self.0.unit_variant()
    }
    fn newtype_variant_seed<T: DeserializeSeed<'de>>(
        self,
        seed: T,
    ) -> Result<T::Value, Self::Error> {
        self.0.newtype_variant_seed(StrictSeed(seed))
    }
    fn tuple_variant<V: Visitor<'de>>(
        self,
        len: usize,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.0.tuple_variant(len, StrictVisitor(visitor))
    }
    fn struct_variant<V: Visitor<'de>>(
        self,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.0.struct_variant(fields, StrictVisitor(visitor))
    }
}
