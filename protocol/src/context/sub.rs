
pub struct EmptyState {}
impl State for EmptyState {}


/*
* return old and parent state
*/

pub fn ctx_state_fork_sub(ctx: &mut dyn Context) -> Arc<Box<dyn State>> {
    let nil = Box::new(EmptyState{});
    let old: Arc<Box<dyn State>> = ctx.state_replace(nil).into();
    let sub = old.fork_sub(Arc::downgrade(&old));
    ctx.state_replace(sub); // drop nil
    old
}


/*

*/
pub fn ctx_state_merge_sub(ctx: &mut dyn Context, old: Arc<Box<dyn State>>) {
    let nil = Box::new(EmptyState{});
    let mut sub = ctx.state_replace(nil);
    sub.detach();
    let mut old = ctx_state_into_box(old);
    old.merge_sub(sub);
    ctx.state_replace(old);
}

pub fn ctx_state_recover(ctx: &mut dyn Context, old: Arc<Box<dyn State>>) {
    ctx.state().detach();
    let old = ctx_state_into_box(old);
    ctx.state_replace(old); // drop sub state
}

pub fn ctx_state_into_box(a: Arc<Box<dyn State>>) -> Box<dyn State> {
    assert_eq!(1, Arc::strong_count(&a));
    assert_eq!(0, Arc::weak_count(&a));
    Arc::into_inner(a).unwrap()
}