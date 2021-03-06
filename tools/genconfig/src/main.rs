// A tool that builds config files for all the nodes and the clients for the
// protocol.

use crypto::{ed25519, secp256k1};
use config::{Node, Client};
use clap::{load_yaml, App};
use types::Replica;
use crypto::Algorithm;
use std::error::Error;
use util::io::*;
use openssl::{asn1::Asn1Time, bn::{BigNum, MsbOption}, error::ErrorStack, hash::MessageDigest, pkey::{PKey, PKeyRef, Private}, rsa::Rsa, x509::{X509, X509NameBuilder, X509Ref, X509Req, X509ReqBuilder, extension::{AuthorityKeyIdentifier, BasicConstraints, KeyUsage, SubjectAlternativeName, SubjectKeyIdentifier}}};
use fnv::FnvHashMap as HashMap;

fn new_root_cert() -> Result<(X509, PKey<Private>), ErrorStack> {
    let rsa = Rsa::generate(2048)?;
    let privkey = PKey::from_rsa(rsa)?;

    let mut x509_name = X509NameBuilder::new()?;
    x509_name.append_entry_by_text("C", "US")?;
    x509_name.append_entry_by_text("ST", "IN")?;
    x509_name.append_entry_by_text("O", "Libchatter Test")?;
    x509_name.append_entry_by_text("CN", "Root")?;
    let x509_name = x509_name.build();

    let mut cert_builder = X509::builder()?;
    cert_builder.set_version(2)?;
    let serial_number = {
        let mut serial = BigNum::new()?;
        serial.rand(159, MsbOption::MAYBE_ZERO, false)?;
        serial.to_asn1_integer()?
    };
    cert_builder.set_serial_number(&serial_number)?;
    cert_builder.set_subject_name(&x509_name)?;
    cert_builder.set_issuer_name(&x509_name)?;
    cert_builder.set_pubkey(&privkey)?;
    let not_before = Asn1Time::days_from_now(0)?;
    cert_builder.set_not_before(&not_before)?;
    let not_after = Asn1Time::days_from_now(365)?;
    cert_builder.set_not_after(&not_after)?;

    cert_builder.append_extension(BasicConstraints::new().critical().ca().build()?)?;
    cert_builder.append_extension(
        KeyUsage::new()
            .critical()
            .key_cert_sign()
            .crl_sign()
            .build()?,
    )?;

    let subject_key_identifier =
        SubjectKeyIdentifier::new().build(&cert_builder.x509v3_context(None, None))?;
    cert_builder.append_extension(subject_key_identifier)?;

    cert_builder.sign(&privkey, MessageDigest::sha256())?;
    let cert = cert_builder.build();

    Ok((cert, privkey))
}

/// Make a X509 request with the given private key
fn mk_request(privkey: &PKey<Private>) -> Result<X509Req, ErrorStack> {
    let mut req_builder = X509ReqBuilder::new()?;
    req_builder.set_pubkey(&privkey)?;

    let mut x509_name = X509NameBuilder::new()?;
    x509_name.append_entry_by_text("C", "US")?;
    x509_name.append_entry_by_text("ST", "IN")?;
    x509_name.append_entry_by_text("O", "Nodes")?;
    x509_name.append_entry_by_text("CN", "nodes.com")?;
    let x509_name = x509_name.build();
    req_builder.set_subject_name(&x509_name)?;

    req_builder.sign(&privkey, MessageDigest::sha256())?;
    let req = req_builder.build();
    Ok(req)
}

/// Make a certificate and private key signed by the given CA cert and private key
fn get_signed_cert(
    ca_cert: &X509Ref,
    ca_privkey: &PKeyRef<Private>,
) -> Result<(X509, PKey<Private>), ErrorStack> {
    let rsa = Rsa::generate(2048)?;
    let privkey = PKey::from_rsa(rsa)?;

    let req = mk_request(&privkey)?;

    let mut cert_builder = X509::builder()?;
    cert_builder.set_version(2)?;
    let serial_number = {
        let mut serial = BigNum::new()?;
        serial.rand(159, MsbOption::MAYBE_ZERO, false)?;
        serial.to_asn1_integer()?
    };
    cert_builder.set_serial_number(&serial_number)?;
    cert_builder.set_subject_name(req.subject_name())?;
    cert_builder.set_issuer_name(ca_cert.subject_name())?;
    cert_builder.set_pubkey(&privkey)?;
    let not_before = Asn1Time::days_from_now(0)?;
    cert_builder.set_not_before(&not_before)?;
    let not_after = Asn1Time::days_from_now(365)?;
    cert_builder.set_not_after(&not_after)?;

    cert_builder.append_extension(BasicConstraints::new().build()?)?;

    cert_builder.append_extension(
        KeyUsage::new()
            .critical()
            .non_repudiation()
            .digital_signature()
            .key_encipherment()
            .build()?,
    )?;

    let subject_key_identifier =
        SubjectKeyIdentifier::new().build(&cert_builder.x509v3_context(Some(ca_cert), None))?;
    cert_builder.append_extension(subject_key_identifier)?;

    let auth_key_identifier = AuthorityKeyIdentifier::new()
        .keyid(false)
        .issuer(false)
        .build(&cert_builder.x509v3_context(Some(ca_cert), None))?;
    cert_builder.append_extension(auth_key_identifier)?;

    let subject_alt_name = SubjectAlternativeName::new()
    //     .dns("*.example.com")
        .dns("nodes.com")
        .build(&cert_builder.x509v3_context(Some(ca_cert), None))?;
    cert_builder.append_extension(subject_alt_name)?;

    cert_builder.sign(&ca_privkey, MessageDigest::sha256())?;
    let cert = cert_builder.build();

    Ok((cert, privkey))
}

fn main() -> Result<(), Box<dyn Error>> {
    let yaml = load_yaml!("cli.yml");
    let m = App::from_yaml(yaml).get_matches();
    let num_nodes:usize =  m.value_of("num_nodes")
        .expect("number of nodes not specified")
        .parse::<usize>()
        .expect("unable to convert number of nodes into a number");
    let num_faults:usize = match m.value_of("num_faults") {
        Some(x) => x.parse::<usize>()
            .expect("unable to convert number of faults into a number"),
        None => (num_nodes-1)/2,
    };
    let delay:u64 = m.value_of("delay")
        .expect("delay value not specified")
        .parse::<u64>()
        .expect("unable to parse delay value into a number");
    let base_port: u16 = m.value_of("base_port")
        .expect("base_port value not specified")
        .parse::<u16>()
        .expect("failed to parse base_port into a number");
    let blocksize: usize = m.value_of("block_size")
        .expect("no block_size specified")
        .parse::<usize>()
        .expect("unable to convert blocksize into a number");
    let client_base_port:u16 = m.value_of("client_base_port")
        .expect("no client_base_port specified")
        .parse::<u16>()
        .expect("unable to parse client_base_port into an integer");
    let t:Algorithm = m.value_of("algorithm")
        .unwrap_or("ED25519")
        .parse::<Algorithm>()
        .unwrap_or(Algorithm::ED25519);
    let out = m.value_of("out_type")
        .unwrap_or("json");
    let target = m.value_of("target")
        .expect("target directory for the config not specified");
    let payload:usize = m.value_of("payload")
        .unwrap_or("0")
        .parse()
        .unwrap();
    let mut client = Client::new();
    client.block_size = blocksize;
    client.crypto_alg = t.clone();
    client.num_nodes = num_nodes;
    client.num_faults = num_faults;

    let mut node:Vec<Node> = Vec::with_capacity(num_nodes);

    let mut pk = HashMap::default();
    let mut ip = HashMap::default();

    let (cert, privkey) = new_root_cert()?;

    for i in 0..num_nodes {
        node.push(Node::new());

        node[i].delta = delay;
        node[i].id = i as Replica;
        node[i].num_nodes = num_nodes;
        node[i].num_faults = num_faults;
        node[i].block_size = blocksize;
        node[i].payload = payload;
        node[i].client_port = client_base_port+(i as u16);

        node[i].crypto_alg = t.clone();
        match t {
            Algorithm::ED25519 => {
                let kp = ed25519::Keypair::generate();
                pk.insert(i as Replica, kp.public().encode().to_vec());
                node[i].secret_key_bytes = kp.encode().to_vec();

            }
            Algorithm::SECP256K1 => {
                let kp = secp256k1::Keypair::generate();
                pk.insert(i as Replica, kp.public().encode().to_vec());
                node[i].secret_key_bytes = kp.secret().to_bytes().to_vec();
            }
            _ => (),
        };
        ip.insert(i as Replica, 
        format!("{}:{}", "127.0.0.1", base_port+(i as u16))
        );
        client.net_map.insert(i as Replica, 
        format!("127.0.0.1:{}", client_base_port+(i as u16))
        );

        let (new_cert, new_pkey) = get_signed_cert(&cert, &privkey)?;

        node[i].root_cert = cert.to_der()?;
        node[i].my_cert = new_cert.to_der()?;
        node[i].my_cert_key = new_pkey.private_key_to_der()?;
    }

    client.root_cert = cert.to_der()?;

    for i in 0..num_nodes {
        node[i].pk_map = pk.clone();
        node[i].net_map = ip.clone();
    }

    client.server_pk = pk;

    // Write all the files
    for i in 0..num_nodes {
        match out {
            "json" => {
                let filename = format!("{}/nodes-{}.json",target,i);
                write_json(filename, &node[i]);
            },
            "binary" => {
                let filename = format!("{}/nodes-{}.dat",target,i);
                write_bin(filename, &node[i]);
            },
            "toml" => {
                let filename = format!("{}/nodes-{}.toml",target,i);
                write_toml(filename, &node[i]);
            },
            "yaml" => {
                let filename = format!("{}/nodes-{}.yml",target,i);
                write_yaml(filename, &node[i]);
            },
            _ => (),
        }
        node[i].validate()
            .expect("failed to validate node config");
    }

    // Write the client file
    match out {
        "json" => {
            let filename = format!("{}/client.json",target);
            write_json(filename, &client);
        },
        "binary" => {
            let filename = format!("{}/client.dat",target);
            write_bin(filename, &client);
        },
        "toml" => {
            let filename = format!("{}/client.toml",target);
            write_toml(filename, &client);
        },
        "yaml" => {
            let filename = format!("{}/client.yml",target);
            write_yaml(filename, &client);
        },
        _ => (),
    }
    client.validate()
        .expect("failed to validate the client config");

    Ok(())
}

#[test]
fn test_codec() -> Result<(), Box<dyn Error>>{
    use rustls::{Certificate, ClientConfig};

    let (cert, _key) = new_root_cert()?;
    let data = cert.to_der()?;
    let ok = Certificate(data);
    let mut config = ClientConfig::new();
    config.root_store.add(&ok)?;
    Ok(())
}