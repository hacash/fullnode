use super::*;
use super::rt::*;


include!{"util.rs"}
include!{"state.rs"}
include!{"exec.rs"}
include!{"ir.rs"}
include!{"execute.rs"}



pub fn do_all_test () {
    codegen1();
    codegen2();
    benchmark1();
    benchmark2();
    execute1();
}




#[cfg(test)]
mod testexec {
    use super::*;
    #[test]
    fn test() {
        benchmark1()
    }
}
/*
ALLOC 2 P0 PUTX 0 GETX 0 P1 EQ BRSL 0 9 P1 PUTX 0 P1 PUTX 1 JMPSL 0 3 P0 PUTX 1 GETX 0 RET 
*/