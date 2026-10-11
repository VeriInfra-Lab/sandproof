#![allow(
    dead_code,
    reason = "shared certificate fixture; suites choose peer cases"
)]
use rcgen::{
    BasicConstraints, Certificate, CertificateParams, ExtendedKeyUsagePurpose, IsCa, Issuer,
    KeyPair, KeyUsagePurpose,
};
use rustls::pki_types::PrivatePkcs8KeyDer;
use sandproof_internal::{CertificatePin, TlsIdentity};

pub struct Identity {
    pub certificate: Certificate,
    pub key: KeyPair,
}

impl Identity {
    pub fn material(&self) -> TlsIdentity {
        TlsIdentity {
            chain: vec![self.certificate.der().clone()],
            private_key: PrivatePkcs8KeyDer::from(self.key.serialize_der()).into(),
        }
    }

    pub fn pin(&self) -> CertificatePin {
        CertificatePin::from_certificate(self.certificate.der())
    }
}

pub struct Certificates {
    pub ca: Certificate,
    pub center: Identity,
    pub host: Identity,
    pub other: Identity,
}

pub fn certificates() -> Certificates {
    let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::DigitalSignature,
    ];
    let ca_key = KeyPair::generate().unwrap();
    let ca = params.self_signed(&ca_key).unwrap();
    let issuer = Issuer::new(params, ca_key);
    Certificates {
        ca,
        center: leaf(&issuer, "center.test", ExtendedKeyUsagePurpose::ClientAuth),
        host: leaf(&issuer, "host.test", ExtendedKeyUsagePurpose::ServerAuth),
        other: leaf(&issuer, "other.test", ExtendedKeyUsagePurpose::ClientAuth),
    }
}

fn leaf(issuer: &Issuer<'_, KeyPair>, name: &str, purpose: ExtendedKeyUsagePurpose) -> Identity {
    let mut params = CertificateParams::new(vec![name.to_owned()]).unwrap();
    params.extended_key_usages = vec![purpose];
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    let key = KeyPair::generate().unwrap();
    Identity {
        certificate: params.signed_by(&key, issuer).unwrap(),
        key,
    }
}
