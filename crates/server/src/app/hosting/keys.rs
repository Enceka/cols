use crate::infra::storage::{atomic_private_write, create_directory, read_bounded, write_private};
use anyhow::{Context, Result, ensure};
use openssl::{
    asn1::Asn1Time,
    bn::{BigNum, MsbOption},
    hash::MessageDigest,
    pkey::PKey,
    rsa::Rsa,
    x509::{
        X509, X509NameBuilder,
        extension::{BasicConstraints, KeyUsage, SubjectAlternativeName},
    },
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    cmp::Ordering,
    path::{Path, PathBuf},
};

#[derive(Serialize)]
pub(crate) struct Certificate {
    pub fingerprint: String,
    pub expires: String,
    pub pem: String,
    #[serde(skip)]
    pub certificate: PathBuf,
    #[serde(skip)]
    pub private_key: PathBuf,
}

pub(super) async fn read(root: &Path) -> Result<Option<Certificate>> {
    let Some(bytes) = read_bounded(&root.join("identity.json"), 4096).await? else {
        return Ok(None);
    };
    let generation: String = serde_json::from_slice(&bytes)?;
    ensure!(
        generation.len() == 32 && generation.bytes().all(|b| b.is_ascii_hexdigit()),
        "本地证书索引无效"
    );
    let directory = root.join("tls").join(generation);
    let certificate = directory.join("server.pem");
    let private_key = directory.join("key.pem");
    let cert = read_bounded(&certificate, 16384)
        .await?
        .context("本地证书缺失")?;
    let key = read_bounded(&private_key, 16384)
        .await?
        .context("本地私钥缺失")?;
    let cert = X509::from_pem(&cert).context("本地证书无效")?;
    let key = PKey::private_key_from_pem(&key).context("本地私钥无效")?;
    ensure!(cert.public_key()?.public_eq(&key), "本地证书与私钥不匹配");
    let now = Asn1Time::days_from_now(0)?;
    ensure!(
        cert.not_before().compare(&now)? != Ordering::Greater
            && cert.not_after().compare(&now)? == Ordering::Greater,
        "本地证书已过期或尚未生效，请在停止服务后更新证书"
    );
    Ok(Some(Certificate {
        fingerprint: format!("{:x}", Sha256::digest(cert.to_der()?)),
        expires: cert.not_after().to_string(),
        pem: String::from_utf8(cert.to_pem()?)?,
        certificate,
        private_key,
    }))
}

pub(super) async fn ensure_identity(root: &Path, renew: bool) -> Result<Certificate> {
    if !renew && let Some(existing) = read(root).await? {
        return Ok(existing);
    }
    let (certificate, key) = tokio::task::spawn_blocking(|| -> Result<_> {
        let key = PKey::from_rsa(Rsa::generate(2048)?)?;
        let mut name = X509NameBuilder::new()?;
        name.append_entry_by_text("CN", "localhost")?;
        let name = name.build();
        let mut cert = X509::builder()?;
        cert.set_version(2)?;
        let mut serial = BigNum::new()?;
        serial.rand(128, MsbOption::MAYBE_ZERO, false)?;
        let serial = serial.to_asn1_integer()?;
        cert.set_serial_number(&serial)?;
        cert.set_subject_name(&name)?;
        cert.set_issuer_name(&name)?;
        cert.set_pubkey(&key)?;
        let before = Asn1Time::days_from_now(0)?;
        let after = Asn1Time::days_from_now(365)?;
        cert.set_not_before(&before)?;
        cert.set_not_after(&after)?;
        cert.append_extension(BasicConstraints::new().critical().ca().build()?)?;
        cert.append_extension(
            KeyUsage::new()
                .critical()
                .digital_signature()
                .key_encipherment()
                .key_cert_sign()
                .build()?,
        )?;
        let san = SubjectAlternativeName::new()
            .dns("localhost")
            .ip("127.0.0.1")
            .build(&cert.x509v3_context(None, None))?;
        cert.append_extension(san)?;
        cert.sign(&key, MessageDigest::sha256())?;
        Ok((cert.build().to_pem()?, key.private_key_to_pem_pkcs8()?))
    })
    .await??;
    let mut random = [0; 16];
    openssl::rand::rand_bytes(&mut random)?;
    let generation = random
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let directory = root.join("tls").join(&generation);
    create_directory(&directory).await?;
    write_private(&directory.join("server.pem"), &certificate).await?;
    write_private(&directory.join("key.pem"), &key).await?;
    for name in ["server.pem", "key.pem"] {
        tokio::fs::File::open(directory.join(name))
            .await?
            .sync_all()
            .await?;
    }
    atomic_private_write(
        &root.join("identity.json"),
        &serde_json::to_vec(&generation)?,
    )
    .await?;
    read(root).await?.context("本地证书创建失败")
}
