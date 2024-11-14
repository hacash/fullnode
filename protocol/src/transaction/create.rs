



macro_rules! transaction_register {
    ( $( $tty:ident )+ ) => {
        
        pub fn create(buf: &[u8]) -> Ret<(Box<dyn Transaction>, usize)> {
            let ty = bufeatone(buf)?;
            match ty {
                $(
                    <$tty>::TYPE => {
                        let (trs, sk) = <$tty>::create(buf)?;
                        Ok((Box::new(trs), sk))
                    },
                )+
                _ => errf!("transaction type '{}' not find", ty)
            }
        }

    };
}



// Trs list
combi_dynvec!{ DynVecTransaction, 
    Uint4, Transaction, create
}





