//! Distinct ID types so a province ID can never be passed as a unit ID.

macro_rules! id_type {
    ($name:ident, $inner:ty) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub $inner);
        impl $name {
            #[inline]
            pub fn index(self) -> usize {
                self.0 as usize
            }
        }
    };
}

id_type!(ProvinceId, u32);
id_type!(CountryId, u16);
id_type!(UnitId, u32);
