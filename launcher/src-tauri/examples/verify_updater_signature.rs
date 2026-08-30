use base64::{engine::general_purpose::STANDARD, Engine};
use minisign_verify::{PublicKey, Signature};
use std::{env, fs, fs::File, io::Read};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args_os().skip(1);
    let artifact = args.next().ok_or("missing updater artifact")?;
    let signature = args.next().ok_or("missing updater signature")?;
    let public_key = args.next().ok_or("missing updater public key")?;
    if args.next().is_some() {
        return Err("unexpected argument".into());
    }

    let public_key = String::from_utf8(STANDARD.decode(fs::read_to_string(public_key)?.trim())?)?;
    let signature = String::from_utf8(STANDARD.decode(fs::read_to_string(signature)?.trim())?)?;
    let public_key = PublicKey::decode(&public_key)?;
    let signature = Signature::decode(&signature)?;
    let mut verifier = public_key.verify_stream(&signature)?;
    let mut artifact = File::open(artifact)?;
    let mut buffer = vec![0; 1024 * 1024];
    loop {
        let read = artifact.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        verifier.update(&buffer[..read]);
    }
    verifier.finalize()?;
    println!("Launcher updater signature verified.");
    Ok(())
}
