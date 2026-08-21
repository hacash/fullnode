//! P2SH wallet / SDK tooling: derives the canonical Merkle tree, `scriptmh` address, and per-leaf
//! proofs / `P2SHScriptProve` actions from `(libs, codeconf, lockbox)` leaves. No consensus mutation.


use field::{Address, BytesW2, Hash, Uint1};

use crate::contract::ContractAddrListW1;
use crate::rt::CodeConf;
#[cfg(feature = "execute")]
use crate::rt::{GasExtra, SpaceCap};

use super::p2sh::{MerkelStuffs, P2SHScriptProve, PosiHash, ScriptmhCalc};

/// A single P2SH leaf: `(adrlibs, codeconf, lockbox)`. Both `adrlibs` and `codeconf`
/// are part of the leaf commitment — differing them changes the `scriptmh` address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P2shLeafSpec {
    pub adrlibs: ContractAddrListW1,
    pub codeconf: CodeConf,
    pub lockbox: BytesW2,
}

#[derive(Debug, Clone)]
pub struct P2shLeaf {
    pub spec: P2shLeafSpec,
    pub leaf_hash: Hash,
}

/// Merkle tree root result.
#[derive(Debug, Clone)]
pub struct P2shTreeCalc {
    pub root_sha3: Hash,
    pub payload20: [u8; 20],
    pub address: Address,
}

/// Canonical Merkle rule: if a level has an odd count, promote the last node to next level.
#[derive(Debug, Clone, Copy, Default)]
pub enum MerkleRule {
    #[default]
    PromoteLastWhenOdd,
}

/// A canonical P2SH Merkle tree (leaves sorted by leaf commitment hash).
#[derive(Debug, Clone)]
pub struct P2shMerkleTree {
    rule: MerkleRule,
    leaves: Vec<P2shLeaf>,  // canonical order
    levels: Vec<Vec<Hash>>, // levels[0] = leaves hashes, levels.last() = [root]
    calc: P2shTreeCalc,
}

/// Tool "class" wrapper: contains the canonical construction algorithms and helpers.
pub struct P2shTool;

impl P2shTool {
    /// Build a canonical Merkle tree from raw leaf specs. Leaves are committed with the
    /// consensus leaf hash, sorted by `leaf_hash` ascending (bytewise); duplicate hashes are rejected.
    pub fn build_canonical_tree(mut specs: Vec<P2shLeafSpec>) -> Ret<P2shMerkleTree> {
        if specs.is_empty() {
            return sys::errf!("p2sh tool: leaf specs cannot be empty");
        }
        let empty_path = MerkelStuffs::from(vec![])?;
        let mut leaves: Vec<P2shLeaf> = Vec::with_capacity(specs.len());
        for spec in specs.drain(..) {
            let calc = P2SHScriptProve::calc_scriptmh_from_lockbox(
                &spec.adrlibs,
                spec.codeconf,
                &spec.lockbox,
                &empty_path,
            )?;
            let leaf_hash = calc.sha3_path[0];
            leaves.push(P2shLeaf { spec, leaf_hash });
        }
        leaves.sort_by(|a, b| a.leaf_hash.as_bytes().cmp(&b.leaf_hash.as_bytes()));
        for i in 1..leaves.len() {
            if leaves[i - 1].leaf_hash == leaves[i].leaf_hash {
                return sys::errf!(
                    "p2sh tool: duplicate leaf hash {}",
                    hex::encode(leaves[i].leaf_hash.as_bytes())
                );
            }
        }
        Self::build_tree_from_sorted_leaves(MerkleRule::PromoteLastWhenOdd, leaves)
    }

    /// Convenience: build a canonical tree from lockboxes that share the same libs + codeconf.
    pub fn build_canonical_tree_shared_libs(
        adrlibs: ContractAddrListW1,
        codeconf: CodeConf,
        lockboxes: Vec<BytesW2>,
    ) -> Ret<P2shMerkleTree> {
        if lockboxes.is_empty() {
            return sys::errf!("p2sh tool: lockbox list cannot be empty");
        }
        let specs: Vec<_> = lockboxes
            .into_iter()
            .map(|lockbox| P2shLeafSpec {
                adrlibs: adrlibs.clone(),
                codeconf,
                lockbox,
            })
            .collect();
        Self::build_canonical_tree(specs)
    }

    fn build_tree_from_sorted_leaves(
        rule: MerkleRule,
        leaves: Vec<P2shLeaf>,
    ) -> Ret<P2shMerkleTree> {
        let mut levels: Vec<Vec<Hash>> = vec![];
        let mut cur: Vec<Hash> = leaves.iter().map(|l| l.leaf_hash).collect();
        levels.push(cur.clone());

        while cur.len() > 1 {
            let mut next: Vec<Hash> = Vec::with_capacity((cur.len() + 1) / 2);
            let mut i = 0usize;
            while i < cur.len() {
                let left = cur[i];
                let right = if i + 1 < cur.len() {
                    cur[i + 1]
                } else {
                    match rule {
                        MerkleRule::PromoteLastWhenOdd => {
                            next.push(left);
                            i += 1;
                            continue;
                        }
                    }
                };
                let mut buf = Vec::with_capacity("p2sh_branch_".len() + 32 + 32);
                buf.extend_from_slice("p2sh_branch_".as_bytes());
                buf.extend_from_slice(left.as_bytes());
                buf.extend_from_slice(right.as_bytes());
                next.push(Hash::from(super::p2sh::p2sh_sha3(buf)));
                i += 2;
            }
            cur = next.clone();
            levels.push(next);
        }

        let root_sha3 = levels.last().unwrap()[0];
        let payload20 = super::p2sh::p2sh_ripemd160(root_sha3.as_bytes());
        let address = super::p2sh::create_scriptmh_addr(payload20);
        Ok(P2shMerkleTree {
            rule,
            leaves,
            levels,
            calc: P2shTreeCalc {
                root_sha3,
                payload20,
                address,
            },
        })
    }
}

impl P2shMerkleTree {
    pub fn address(&self) -> Address {
        self.calc.address
    }
    pub fn root_sha3(&self) -> Hash {
        self.calc.root_sha3
    }
    pub fn leaves(&self) -> &Vec<P2shLeaf> {
        &self.leaves
    }
    pub fn merkle_rule(&self) -> MerkleRule {
        self.rule
    }

    /// Return the Merkle proof path (siblings + posi) for the leaf at canonical index `idx`.
    /// `posi` matches consensus `get_merkel()`: `0`=sibling on LEFT, `1`=sibling on RIGHT.
    pub fn proof_for_index(&self, idx: usize) -> Ret<MerkelStuffs> {
        if idx >= self.leaves.len() {
            return sys::errf!(
                "p2sh tool: leaf index {} overflow (len={})",
                idx,
                self.leaves.len()
            );
        }
        let mut path: Vec<PosiHash> = vec![];
        let mut i = idx;
        // levels[0] is leaf level, levels.last() is root level (len==1)
        for level in &self.levels[..self.levels.len() - 1] {
            let n = level.len();
            let sib = if i % 2 == 0 {
                if i + 1 < n {
                    Some((i + 1, 1u8)) // sibling on the right
                } else {
                    None // odd tail was promoted
                }
            } else {
                Some((i - 1, 0u8)) // sibling on the left
            };
            if let Some((sib_idx, posi)) = sib {
                path.push(PosiHash {
                    posi: Uint1::from(posi),
                    hash: level[sib_idx],
                });
            }
            i /= 2;
        }
        MerkelStuffs::from(path)
    }

    pub fn select_index_by_leaf_hash(&self, leaf_hash: &Hash) -> Ret<usize> {
        self.leaves
            .iter()
            .position(|l| &l.leaf_hash == leaf_hash)
            .ok_or_else(|| {
                sys::Error::fault(format!(
                    "p2sh tool: leaf hash {} not found",
                    hex::encode(leaf_hash.as_bytes())
                ))
            })
    }

    pub fn select_index_by_spec(
        &self,
        adrlibs: &ContractAddrListW1,
        codeconf: CodeConf,
        lockbox: &BytesW2,
    ) -> Ret<usize> {
        self.leaves
            .iter()
            .position(|l| {
                &l.spec.adrlibs == adrlibs
                    && l.spec.codeconf == codeconf
                    && &l.spec.lockbox == lockbox
            })
            .ok_or_else(|| {
                sys::Error::fault("p2sh tool: leaf (libs, codeconf, lockbox) not found".to_owned())
            })
    }

    /// Build a `P2SHScriptProve` action for the leaf at `idx`: returns the `scriptmh` address (use as
    /// `from`), the filled action, and `ScriptmhCalc`; no bytecode validation here (chain validates in execute).
    pub fn build_unlock_script_prove_unchecked(
        &self,
        idx: usize,
        witness: BytesW2,
    ) -> Ret<(Address, P2SHScriptProve, ScriptmhCalc)> {
        let spec = self
            .leaves
            .get(idx)
            .ok_or_else(|| sys::Error::fault(format!("p2sh tool: leaf index {} overflow", idx)))?
            .spec
            .clone();
        let merkels = self.proof_for_index(idx)?;
        let calc = P2SHScriptProve::calc_scriptmh_from_lockbox(
            &spec.adrlibs,
            spec.codeconf,
            &spec.lockbox,
            &merkels,
        )?;
        if calc.address != self.calc.address {
            return sys::errf!(
                "p2sh tool: proof derived address {} mismatch tree address {}",
                calc.address.to_readable(),
                self.calc.address.to_readable()
            );
        }
        let mut act = P2SHScriptProve::new();
        act.argvkey = witness;
        act.adrlibs = spec.adrlibs;
        act.codeconf = Uint1::from(spec.codeconf.raw());
        act.lockbox = spec.lockbox;
        act.merkels = merkels;
        // `marks` keeps default zero, which passes the on-chain check.
        Ok((calc.address, act, calc))
    }

    /// Same as `build_unlock_script_prove_unchecked`, but locally checks `get_stuff` rules and
    /// Merkle proof depth against `SpaceCap::p2sh_merkle_depth_max` (matches on-chain `execute`).
    #[cfg(feature = "execute")]
    pub fn build_unlock_script_prove_checked(
        &self,
        block_height: u64,
        idx: usize,
        witness: BytesW2,
        registry: &dyn base::ExecutionServices,
    ) -> Ret<(Address, P2SHScriptProve, ScriptmhCalc)> {
        let spec = self
            .leaves
            .get(idx)
            .ok_or_else(|| sys::Error::fault(format!("p2sh tool: leaf index {} overflow", idx)))?
            .spec
            .clone();
        let gst = GasExtra::new(block_height);
        let cap = SpaceCap::new(block_height);
        P2SHScriptProve::verify_unlock_inputs(
            block_height,
            &gst,
            &cap,
            &spec.adrlibs,
            spec.codeconf,
            &spec.lockbox,
            &witness,
            registry,
        )?;
        let merkle_depth = self.proof_for_index(idx)?.as_list().len();
        P2SHScriptProve::verify_merkle_depth(&cap, merkle_depth)?;
        self.build_unlock_script_prove_unchecked(idx, witness)
    }
}

// Re-echo the sys::Ret import used across the module's public API.
use sys::Ret;
