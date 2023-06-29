use cid::{multibase::Base, multihash::Multihash, Cid};
use ring::digest::{Digest, Context, SHA256};

// SHA2_256
pub const CURR_MULTIHASH_FORMAT: u64 = 0x12;
pub const CURR_CODEC: u64 = 0x55;
pub const CURR_MULTIBASE_FORMAT: Base = Base::Base32Lower;

pub fn wrap_digest(d: Digest) -> Result<Vec<u8>, cid::multihash::Error> {
    let m_hash = Multihash::wrap(CURR_MULTIHASH_FORMAT, d.as_ref())?;
    Ok(Cid::new_v1(CURR_CODEC, m_hash).into())
}

pub fn encode<T>(t: T) -> String
where
    T: AsRef<[u8]>,
{
    cid::multibase::encode(CURR_MULTIBASE_FORMAT, t)
}


pub fn new_digest_context() -> Context {
    Context::new(&SHA256)
}
