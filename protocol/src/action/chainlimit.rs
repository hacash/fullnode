
/*
*
*/
action_define!{ SubmitHeightLimit, 29, 
    ActLv::TopUnique, // level
    false, // burn 90 fee
    [], // need sign
    {
        start: BlockHeight
        end:   BlockHeight
    },
    (self, ctx, _gas {
        let pdhei = ctx.env().block.height;
        let left = *self.start;
        let right = match *self.end {
            0 => u64::MAX,
            h => h,
        };
        if left > right {
            return errf!("left height {} cannot big than rigth height {}", left, right)
        }
        if pdhei < left || pdhei > right {
            return errf!("transction must submit in height between {} and {}", left, right)
        }
        // ok
        Ok(vec![])
    })
}




action_define!{ SubChainID, 30, 
    ActLv::TopUnique, // level
    false, // burn 90 fee
    [], // need sign
    {
        chain_id: Uint4
    },
    (self, ctx, _gas {
        let lid = ctx.env().chain.id;
        let sid = *self.chain_id;
        if lid != sid {
            return errf!("transction must belong to chain id {} but on chain {}", sid, lid)
        }
        // ok
        Ok(vec![])
    })
}
