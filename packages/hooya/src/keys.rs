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
    sign, verify, FieldExtensionTrait, Fp, Fr, G1Affine, G1Projective,
    G2Affine, G2Projective, GroupTrait, KeyPair,
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
pub fn keypair_pubkey_raw_bytes(kp: &KeyPair) -> [u8; 128] {
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
    let signature = sign(&kp.secret_key, message).expect("Signing failed");
    // to bytes because G1Projective is hard to work w
    G1Affine::from(signature).to_be_bytes().to_vec()
}

/// Verify a BLS signature using the public key
pub fn verify_signature(
    pubkey_bytes: &[u8],
    message: &[u8],
    signature_bytes: &[u8],
) -> anyhow::Result<bool> {
    if pubkey_bytes.len() != 128 {
        return Ok(false);
    }

    let mut pubkey_array = [0u8; 128];
    pubkey_array.copy_from_slice(pubkey_bytes);

    // reverse the coordinate swapping from keypair_pubkey_raw_bytes
    let (x, y) = pubkey_array.split_at_mut(64);
    let (x1, x2) = x.split_at_mut(32);
    let (y1, y2) = y.split_at_mut(32);

    x2.swap_with_slice(x1);
    y2.swap_with_slice(y1);

    let public_key = G2Affine::from_be_bytes(&pubkey_array)
        .into_option()
        .ok_or_else(|| anyhow::anyhow!("invalid public key bytes"))?;

    // reconstruct signature from bytes
    if signature_bytes.len() != 64 {
        return Ok(false);
    }

    let mut sig_array = [0u8; 64];
    sig_array.copy_from_slice(signature_bytes);

    let signature = G1Affine::from_be_bytes(&sig_array)
        .into_option()
        .ok_or_else(|| anyhow::anyhow!("invalid signature bytes"))?;

    verify(&public_key, message, &G1Projective::from(signature))
        .map_err(|e| anyhow::anyhow!("verification failed: {:?}", e))
}

/// Derive a discv5 secp256k1 key from the BLS keypair
pub fn derive_discv5_key(
    bls_keypair: &KeyPair,
) -> anyhow::Result<discv5::enr::CombinedKey> {
    let secret_bytes = bls_keypair.secret_key.to_be_bytes();
    let secp256k1_key =
        discv5::enr::k256::SecretKey::from_bytes((&secret_bytes).into())
            .map_err(|e| {
                anyhow::anyhow!("Failed to create secp256k1 key: {}", e)
            })?;
    Ok(discv5::enr::CombinedKey::Secp256k1(secp256k1_key.into()))
}

/// Derive a libp2p keypair from the BLS keypair
/// Uses ed25519 since secp256k1 feature may not be enabled
pub fn derive_libp2p_key(
    bls_keypair: &KeyPair,
) -> anyhow::Result<libp2p::identity::Keypair> {
    let secret_bytes = bls_keypair.secret_key.to_be_bytes();
    // Use the first 32 bytes for ed25519 key generation
    let mut ed25519_bytes = [0u8; 32];
    ed25519_bytes.copy_from_slice(&secret_bytes[..32]);
    let ed25519_key =
        libp2p::identity::ed25519::SecretKey::try_from_bytes(ed25519_bytes)
            .map_err(|e| {
                anyhow::anyhow!("Failed to create ed25519 key: {}", e)
            })?;
    let keypair = libp2p::identity::ed25519::Keypair::from(ed25519_key);
    Ok(libp2p::identity::Keypair::from(keypair))
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

        assert_eq!(node_id.len(), 31);
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

    #[test]
    fn test_signature_verification() {
        let temp_dir = TempDir::new().unwrap();
        let key_path = temp_dir.path().join("test-key");
        let kp = write_secret_bls_key_at_path(&key_path).unwrap();

        let message = b"test message";
        let signature = sign_message(&kp, message);
        let pubkey_bytes = keypair_pubkey_raw_bytes(&kp);

        // valid signature should verify
        assert!(verify_signature(&pubkey_bytes, message, &signature).unwrap());

        // tampered message should not verify
        let tampered_message = b"tampered message";
        assert!(
            !verify_signature(&pubkey_bytes, tampered_message, &signature)
                .unwrap()
        );

        // tampered signature should not verify
        let mut tampered_signature = signature.clone();
        tampered_signature[0] ^= 1;
        let tampered_result =
            verify_signature(&pubkey_bytes, message, &tampered_signature);
        assert!(tampered_result.is_err() || !tampered_result.unwrap());
    }
}
