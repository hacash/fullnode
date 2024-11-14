/*
* type 
*/
pub type ArcDynState = Arc<dyn State>;



pub struct ActLv {}
impl ActLv {

    pub const TOP_ONLY:     i8 =  -4; // only this single one on top
    pub const TOP_UNIQUE:   i8 =  -3; // top and unique
    pub const TOP:          i8 =  -2; // must on top
    pub const CONDAST:      i8 =  -1; // on act cond AST 
    pub const MAINCALL:     i8 =   0; // must in tx main call with depth 0
    pub const ANY:          i8 = 127; // any where

}






