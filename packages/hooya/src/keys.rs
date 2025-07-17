use multibase::Base;
use rand::thread_rng;
use sha2::{Digest, Sha256};
use std::io::Read as _;
use std::{
    fs::File,
    io::Write as _,
    path::{Path, PathBuf},
};
use sylow::{
    FieldExtensionTrait, Fp, Fr, G2Affine, G2Projective, GroupTrait, KeyPair,
};

pub const DEFAULT_MULTIBASE_BASE: Base = multibase::Base::Base58Btc;

/// Generate or load a BLS keypair from a file path
pub fn secret_bls_key_at_path(file: &PathBuf) -> anyhow::Result<KeyPair> {
    let secret_key = if file.exists() {
        let mut file = File::open(file)?;
        let mut key_data = String::new();
        file.read_to_string(&mut key_data)?;

        let (_, decoded_bytes) = multibase::decode(&key_data)?;
        let byte_array: [u8; 32] =
            decoded_bytes.as_slice().try_into().map_err(|_| {
                anyhow::anyhow!("decoded privkey is not 32 bytes long")
            })?;

        Fp::from_be_bytes(&byte_array).into_option().ok_or_else(|| {
            anyhow::anyhow!("failed to convert decoded bytes to privkey")
        })
    } else {
        return Err(anyhow::anyhow!(
            "no secret key at path: {}",
            file.display()
        ));
    }?;

    let public_key = G2Projective::generator() * secret_key;

    Ok(KeyPair {
        secret_key,
        public_key,
    })
}

/// Generate a new BLS keypair and write it to a file
pub fn write_secret_bls_key_at_path(file: &PathBuf) -> anyhow::Result<KeyPair> {
    let mut rng = thread_rng();
    let secret_key = Fp::new(Fr::rand(&mut rng).value());

    let mut file = File::create(file)?;
    let encoded_key =
        multibase::encode(DEFAULT_MULTIBASE_BASE, secret_key.to_be_bytes());
    file.write_all(encoded_key.as_bytes())?;

    let public_key = G2Projective::generator() * secret_key;
    Ok(KeyPair {
        secret_key,
        public_key,
    })
}

/// Generate or load a BLS keypair, creating it if it doesn't exist
pub fn get_or_create_bls_key_at_path(
    file: &PathBuf,
) -> anyhow::Result<KeyPair> {
    if file.exists() {
        secret_bls_key_at_path(file)
    } else {
        // ensure parent directory exists
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        write_secret_bls_key_at_path(file)
    }
}

/// Convert a BLS public key to raw bytes (128 bytes)
fn keypair_pubkey_raw_bytes(kp: &KeyPair) -> [u8; 128] {
    let mut point = G2Affine::from(kp.public_key).to_be_bytes();

    // I can't remember why I flipped these but it was important
    let (x, y) = point.split_at_mut(64);
    let (x1, x2) = x.split_at_mut(32);
    let (y1, y2) = y.split_at_mut(32);

    x2.swap_with_slice(x1);
    y2.swap_with_slice(y1);

    point
}

/// Convert a BLS public key to multibase string
pub fn keypair_pubkey_to_hex(kp: &KeyPair) -> String {
    multibase::encode(DEFAULT_MULTIBASE_BASE, keypair_pubkey_raw_bytes(kp))
}

/// Derives a node ID from a BLS public key
pub fn derive_node_id(kp: &KeyPair) -> String {
    let pubkey_bytes = keypair_pubkey_raw_bytes(kp);
    let mut hasher = Sha256::new();
    hasher.update(pubkey_bytes);
    let hash = hasher.finalize();

    // use first 20 bytes for node ID then encode with multibase
    let node_id_bytes = &hash[..20];
    format!(
        "0x{}",
        multibase::encode(DEFAULT_MULTIBASE_BASE, node_id_bytes)
    )
}

/// Sign a message using the BLS keypair
pub fn sign_message(kp: &KeyPair, message: &[u8]) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(message);
    let hash = hasher.finalize();

    // Convert hash to Fr scalar for BLS signature
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(&hash[..32]);
    let scalar =
        Fr::from_be_bytes(&bytes).unwrap_or_else(|| Fr::new(0u64.into()));

    // BLS signature: scalar * secret_key
    let signature = kp.secret_key * scalar.into();

    signature.to_be_bytes().to_vec()
}

/// load and optionally initialize the node and consensus keys
///
/// node-secret is an identifier and signing key for the node
/// consensus-secret is an (unused) signing key used in network consensus
pub fn initialize_keys(
    filestore_path: &Path,
) -> anyhow::Result<(KeyPair, Option<KeyPair>)> {
    let node_secret_path = filestore_path.join("node-secret");
    let consensus_secret_path = filestore_path.join("consensus-secret");

    // node-secret
    let node_keypair = get_or_create_bls_key_at_path(&node_secret_path)?;

    // consensus-secret
    let consensus_keypair = if consensus_secret_path.exists() {
        Some(secret_bls_key_at_path(&consensus_secret_path)?)
    } else {
        None
    };

    Ok((node_keypair, consensus_keypair))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_key_generation_and_loading() {
        let temp_dir = TempDir::new().unwrap();
        let key_path = temp_dir.path().join("test-key");

        // generate a new key
        let kp1 = write_secret_bls_key_at_path(&key_path).unwrap();

        // load the same key
        let kp2 = secret_bls_key_at_path(&key_path).unwrap();

        // they should be the same
        assert_eq!(kp1.secret_key, kp2.secret_key);
        assert_eq!(kp1.public_key, kp2.public_key);
    }

    #[test]
    fn test_node_id_derivation() {
        let temp_dir = TempDir::new().unwrap();
        let key_path = temp_dir.path().join("test-key");

        let kp = write_secret_bls_key_at_path(&key_path).unwrap();
        let node_id = derive_node_id(&kp);

        // node ID should be 42 characters (0x + 40 hex chars)
        assert_eq!(node_id.len(), 42);
        assert!(node_id.starts_with("0x"));
    }

    #[test]
    fn test_get_or_create_bls_key() {
        let temp_dir = TempDir::new().unwrap();
        let key_path = temp_dir.path().join("test-key");

        // first call should create the key
        let kp1 = get_or_create_bls_key_at_path(&key_path).unwrap();

        // second call should load the same key
        let kp2 = get_or_create_bls_key_at_path(&key_path).unwrap();

        assert_eq!(kp1.secret_key, kp2.secret_key);
        assert_eq!(kp1.public_key, kp2.public_key);
    }

    #[test]
    fn test_initialize_keys() {
        let temp_dir = TempDir::new().unwrap();
        let filestore_path = temp_dir.path().to_path_buf();

        let (node_kp, consensus_kp) = initialize_keys(&filestore_path).unwrap();

        // node key should exist, consensus key should not
        assert!(filestore_path.join("node-secret").exists());
        assert!(!filestore_path.join("consensus-secret").exists());
        assert!(consensus_kp.is_none());

        // create a consensus key manually for testing
        let consensus_path = filestore_path.join("consensus-secret");
        let consensus_kp_created =
            write_secret_bls_key_at_path(&consensus_path).unwrap();

        // now initialize_keys should load both
        let (node_kp2, consensus_kp2) =
            initialize_keys(&filestore_path).unwrap();
        assert_eq!(node_kp.secret_key, node_kp2.secret_key);
        assert!(consensus_kp2.is_some());
        assert_eq!(
            consensus_kp_created.secret_key,
            consensus_kp2.unwrap().secret_key
        );
    }
}
