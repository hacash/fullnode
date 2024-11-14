

// create macro
#[macro_export] 
macro_rules! combi_option {
    ($class:ident, $vty:ty) => (


        #[derive(Default, Clone, PartialEq, Eq)]
        pub struct $class {
            exist: Bool,
            value: Option<$vty>,
        }

        impl std::fmt::Debug for $class {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f,"[ifval]")
            }
        }

        impl Parse for $class {

            fn parse(&mut self, buf: &[u8]) -> Ret<usize> {
                // println!("{}", hex::encode(buf));
                // println!("StructFieldOptional parse exist {} {}", buf.len(), seek);
                let mut seek = self.exist.parse(buf) ?;
                // println!("StructFieldOptional parse {}", seek);
                if self.is_exist() {
                    let (val, mvsk) = <$vty>::create(&buf[seek..]) ?;
                    self.value = Some(val);
                    seek += mvsk
                }
                Ok(seek)
            }
        }

        impl Serialize for $class {

            fn serialize(&self) -> Vec<u8> {
                let mut resdt = self.exist.serialize();
                if self.is_exist() {
                    let mut vardt = self.value.as_ref().unwrap().serialize();
                    resdt.append(&mut vardt);
                }
                resdt
            }

            fn size(&self) -> usize {
                let mut size = self.exist.size();
                if self.is_exist() {
                    size += self.value.as_ref().unwrap().size();
                }
                size
            }

        }

        impl_field_only_new!{$class}


        impl $class {
            
            pub fn is_exist(&self) -> bool {
                self.exist.check()
            }

            pub fn must(v: $vty) -> $class {
                $class {
                    exist: Bool::from(true),
                    value: Some(v),
                }
            }

            pub fn from_value(ifv: Option<$vty>) -> $class {
                match ifv {
                    Some(v) => <$class>::must(v),
                    _ => <$class>::default(),
                }
            }

            pub fn if_value(&self) -> Option<& $vty> {
                match &self.value {
                    Some(v) => Some(&v),
                    None => None,
                }
            }
            
            // clone
            pub fn value(&self) -> $vty {
                match self.exist.check() {
                    true => self.value.as_ref().unwrap().clone(),
                    false => <$vty>::default(),
                }
            }
            

        }




    )
}
