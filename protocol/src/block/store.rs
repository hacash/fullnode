
    
pub fn load_block(store: &dyn Store, hx: &Hash) -> Option<(Vec<u8>, Box<dyn Block>)> {
	let Some(data) = store.block_data(&hx) else {
		return None
	};
	// parse
	match block::create(&data).map(|(b,_)|b) {
		Err(..) => None,
		Ok(b) => Some((data, b))
	}
}
    


pub fn load_block_by_height(store: &dyn Store, hei: &BlockHeight) -> Option<(Hash, Vec<u8>, Box<dyn Block>)> {
	let Some(hx) = store.block_hash(hei) else {
		return None
	};
	let Some((data, block)) = load_block(store, &hx) else {
		return None
	};
	Some((hx, data, block))
}


pub fn build_block_package(data: Vec<u8>) -> Ret<BlockPkg> {
    let (objc, _) = block::create(&data)?;
    Ok(BlockPkg {
        orgi: BlkOrigin::Unknown,
        hein: objc.height().uint(),
        hash: objc.hash(),
        data,
        objc,
    })
}