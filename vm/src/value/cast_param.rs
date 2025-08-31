
/**
* 
*/

pub fn cast_arithmetic(x: &mut Value, y: &mut Value) -> VmrtErr {

    match (&x, &y) {
        (U8(_),   U8(_)) |
        (U16(_),  U16(_)) |
        (U32(_),  U32(_)) |
        (U64(_),  U64(_)) |
        (U128(_), U128(_)) => return Ok(()), // no need

        (U8(l),   U16(_)) =>  { *x = U16((*l).into()) },
        (U8(l),   U32(_)) =>  { *x = U32((*l).into()) },
        (U8(l),   U64(_)) =>  { *x = U64((*l).into()) },
        (U8(l),   U128(_)) => { *x = U128((*l).into()) },

        (U16(_),   U8(r)) =>   { *y = U16((*r).into()) },
        (U16(l),   U32(_)) =>  { *x = U32((*l).into()) },
        (U16(l),   U64(_)) =>  { *x = U64((*l).into()) },
        (U16(l),   U128(_)) => { *x = U128((*l).into()) },

        (U32(_),   U8(r)) =>   { *y = U32((*r).into()) },
        (U32(_),   U16(r)) =>  { *y = U32((*r).into()) },
        (U32(l),   U64(_)) =>  { *x = U64((*l).into()) },
        (U32(l),   U128(_)) => { *x = U128((*l).into()) },

        (U64(_),   U8(r)) =>   { *y = U64((*r).into()) },
        (U64(_),   U16(r)) =>  { *y = U64((*r).into()) },
        (U64(_),   U32(r)) =>  { *y = U64((*r).into()) },
        (U64(l),   U128(_)) => { *x = U128((*l).into()) },

        (U128(_),   U8(r)) =>  { *y = U128((*r).into()) },
        (U128(_),   U16(r)) => { *y = U128((*r).into()) },
        (U128(_),   U32(r)) => { *y = U128((*r).into()) },
        (U128(_),   U64(r)) => { *y = U128((*r).into()) },

        (Bytes(l), Bytes(r)) => {
            *x = buf_to_uint(&l)?;
            *y = buf_to_uint(&r)?;
            return cast_arithmetic(x, y)
        },
        (Bytes(l), _) => { 
            *x = buf_to_uint(&l)?;
            return cast_arithmetic(x, y)
        },
        (_, Bytes(r)) => { 
            *y = buf_to_uint(&r)?;
            return cast_arithmetic(x, y)
        },

        (l, r) => return itr_err_fmt!(CastFail, 
            "cannot do arithmetic cast between type {:?} and {:?}", l, r),
    };

    Ok(())

}