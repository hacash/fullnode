
pub fn create(buf: &[u8]) -> Ret<(Box<dyn Block>, usize)> {
    // println!("block::create {}", hex::encode(buf));
    let version = bufeatone(buf)?;
    match version {
        BlockV1::VERSION => {
            let (blk, mvsk) = BlockV1::create(buf)?;
            Ok((Box::new(blk), mvsk))
        }
        _ => errf!("block version '{}' not find", version)
    }
}


/*
pub fn create_pkg(bytes: BytesW4) -> Ret<Box<dyn BlockPkg>> {
    let buf = bytes.as_ref();
    let (blkobj, _) = create(buf)?;
    let hash = blkobj.hash();
    Ok(Box::new(BlockPackage::new_with_data(blkobj, bytes.into_vec())))
}
*/
