


pub fn build_tx_package(data: Vec<u8>) -> Ret<TxPkg> {
    let (objc, _) = transaction::create(&data)?;
    let mut pkg = TxPkg {
        orgi: TxOrigin::Unknown,
        hash: objc.hash(),
        fepr: 0,
        data,
        objc,
    };
    pkg.fepr = pkg.calc_fee_purity();
    Ok(pkg)
}


