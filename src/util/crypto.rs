use jsonwebtoken as jwt;
use ring::signature::KeyPair;

pub fn parse_keypair<B: AsRef<[u8]>>(
    pem_data: B,
) -> Result<(jwt::EncodingKey, jwt::DecodingKey<'static>), Box<dyn std::error::Error>> {
    // TODO: validate it is a prime256v1 PKCS#8 PEM encoded private key

    let content = pem::parse(pem_data)?;
    let keypair = ring::signature::EcdsaKeyPair::from_pkcs8(
        &ring::signature::ECDSA_P256_SHA256_FIXED_SIGNING,
        &content.contents,
    )?;
    Ok((
        jwt::EncodingKey::from_ec_der(&content.contents),
        jwt::DecodingKey::from_ec_der(keypair.public_key().as_ref()).into_static(),
    ))
}
