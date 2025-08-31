use std::ops::DerefMut;


#[derive(Default)]
struct AstNull {}

impl AST for AstNull {
    fn as_any(&self) -> &dyn Any { self }
    fn is_null(&self) -> bool { true }
}

impl Default for Box<dyn AST> {
    fn default() -> Box<dyn AST> {
        Box::new(AstNull{})
    }
}

impl AstNull {
    pub fn null() -> Box<dyn AST> {
        Box::new(AstNull{})
    }
}

/*************************************************/

#[derive(Default)]
struct AstBlock {
    lists: Vec<Box<dyn AST>>,
}

impl AST for AstBlock {
    fn as_any(&self) -> &dyn Any { self }
}

impl std::ops::Deref for AstBlock {
    type Target = Vec<Box<dyn AST>>;
    fn deref(&self) -> &Vec<Box<dyn AST>> {
        &self.lists
    }
}

impl DerefMut for AstBlock {
    fn deref_mut(&mut self) -> &mut Vec<Box<dyn AST>> {
        &mut self.lists
    }
}

impl AstBlock {
}


/*************************************************/


struct AstAssign {
    name: String,
    value: Box<dyn AST>,
}

impl AST for AstAssign {
    fn as_any(&self) -> &dyn Any { self }
}

impl AstAssign {
    pub fn build(n: &str, v: Box<dyn AST>) -> Ret<Self> {
        if ! v.expression() {
            return errf!("AstAssign sub not expression")
        }
        Ok(Self {
            name: n.to_string(),
            value: v,
        })
    }
}


/*************************************************/


enum AstLeaf {
    Int(u128),
    Buf(String),
    Var(String),
}

impl AST for AstLeaf {
    fn as_any(&self) -> &dyn Any { self }
    fn expression(&self) -> bool { true }
}

impl AstLeaf {
    pub fn int(v: u128) -> Self {
        Self::Int(v)
    }
    pub fn buf(v: &str) -> Self {
        Self::Buf(v.to_owned())
    }
    pub fn var(v: &str) -> Self {
        Self::Var(v.to_owned())
    }

    pub fn must_identifier(d: &Box<dyn AST>) -> Ret<String> {
        let Some(key) = d.as_any().downcast_ref::<AstLeaf>() else {
            return errf!("unsupport assign")
        };
        match key {
            Self::Var(id) => Ok(id.to_string()),
            _ => errf!("unsupport assign")
        }
    }
}




struct AstOperand {
    pub opty: OpTy,
    pub left: Box<dyn AST>,
    pub right: Box<dyn AST>,
}

impl AST for AstOperand {
    fn as_any(&self) -> &dyn Any { self }
    fn expression(&self) -> bool { true }
}

impl AstOperand {
    pub fn build(opty: OpTy, left: Box<dyn AST>, right: Box<dyn AST>) -> Ret<Self> {
        if ! left.expression() {
            return errf!("AstOperand left sub not expression")
        }
        if ! right.expression() {
            return errf!("AstOperand right sub not expression")
        }
        Ok(Self{ opty, left, right })
    }
}





struct AstChild {
    pub opty: OpTy,
    pub subv: Box<dyn AST>,
}

impl AST for AstChild {
    fn as_any(&self) -> &dyn Any { self }
    fn expression(&self) -> bool {
        false
    }
}

impl AstChild {
    pub fn build(opty: OpTy, subv: Box<dyn AST>) -> Ret<Self> {
        if ! subv.expression() {
            return errf!("AstChild sub not expression")
        }
        Ok(Self{ opty, subv })
    }
}



