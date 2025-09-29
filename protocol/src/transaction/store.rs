


pub fn build_tx_package(data: Vec<u8>) -> Ret<TxPkg> {
    let (objc, _) = transaction::create(&data)?;
    let pkg = TxPkg {
        orgi: TxOrigin::Unknown,
        hash: objc.hash(),
        fepr: objc.fee_purity(),
        data,
        objc,
    };
    Ok(pkg)
}


