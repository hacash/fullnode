
pub struct ActionLevel {}
impl ActionLevel {

    pub const TOP_ONLY:     i8 =   -3; // only this single one on top
    pub const TOP_UNIQUE:   i8 =   -2; // top and unique
    pub const TOP:          i8 =   -1; // must on top
    pub const MAIN:         i8 =    0; // must in tx main call with depth 0
    pub const ANY:          i8 =  127; // any where

}




