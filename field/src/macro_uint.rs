
macro_rules! from_uint {
    ($ty:ident, $vt: ty) => (
        concat_idents!{frn = from_, $ty {
            fn frn(v: $ty) -> Self {
                Self{ value: v as $vt}
            }
        }}
    )
}


macro_rules! parse_uint {
    ($ty:ident, $vt: ty) => (
        concat_idents!{frn = parse_, $ty {
            fn frn(&mut self, v: $ty) -> Rerr {
                self.value = v as $vt;
                Ok(())
            }
        }}
    )
}


