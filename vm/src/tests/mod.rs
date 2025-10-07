use super::*;
use super::rt::*;
use super::lang::*;


include!{"util.rs"}
include!{"state.rs"}
include!{"benchmark.rs"}
include!{"ir.rs"}
include!{"execute.rs"}



#[allow(dead_code)]
pub fn do_all_test () {
    codegen1();
    codegen2();
    benchmark1();
    benchmark2();
    execute1();
    execute2();
    execute3();
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
ALLOC 2 P0 PUT 0 GETX 0 P1 EQ BRSL 0 9 P1 PUT 0 P1 PUT 1 JMPSL 0 3 P0 PUT 1 GETX 0 RET 
*/