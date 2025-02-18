use super::utils::CHAIN_ID_TO_FORWARDER;
use ethers::abi::{self, Token};
use ethers::types::{transaction::eip712::*, Address};
use ethers::utils::hex::FromHexError;
use ethers::utils::keccak256;
use gelato_relay::ForwardRequest;
use std::str::FromStr;

const FORWARD_REQUEST_TYPE: &str = "ForwardRequest(uint256 chainId,address target,bytes data,address feeToken,uint256 paymentType,uint256 maxFee,uint256 gas,address sponsor,uint256 sponsorChainId,uint256 nonce,bool enforceSponsorNonce,bool enforceSponsorNonceOrdering)";

/// Unfilled Gelato forward request. This request is signed and filled according
/// to EIP-712 then sent to Gelato. Gelato executes the provided tx `data` on
/// the `target` contract address.
#[derive(Debug, Clone)]
pub struct UnfilledForwardRequest {
    /// Target chain id
    pub chain_id: usize,
    /// Target contract address
    pub target: String,
    /// Encoded tx data
    pub data: String,
    /// Fee token address
    pub fee_token: String,
    /// Payment method
    pub payment_type: usize, // 1 = gas tank
    /// Max fee
    pub max_fee: usize,
    /// Contract call gas limit + buffer for gelato forwarder
    pub gas: usize,
    /// Sponsor address
    pub sponsor: String,
    /// Sponsor resident chain id
    pub sponsor_chain_id: usize, // same as chain_id
    /// Nonce for replay protection
    pub nonce: usize, // can default 0 if next field false
    /// Enforce nonce replay protection
    pub enforce_sponsor_nonce: bool, // default false given replay safe
    /// Enforce ordering based on provided nonces. Only considered if
    /// `enforce_sponsor_nonce` true.
    pub enforce_sponsor_nonce_ordering: bool,
}

/// ForwardRequest error
#[derive(Debug, thiserror::Error, Clone, Copy)]
pub enum ForwardRequestError {
    /// Hex decoding error
    #[error("Hex decoding error: {0}")]
    FromHexError(#[from] FromHexError),
}

impl Eip712 for UnfilledForwardRequest {
    type Error = ForwardRequestError;

    fn domain(&self) -> Result<EIP712Domain, Self::Error> {
        Ok(EIP712Domain {
            name: "GelatoRelayForwarder".to_owned(),
            version: "V1".to_owned(),
            chain_id: self.chain_id.into(),
            verifying_contract: *CHAIN_ID_TO_FORWARDER
                .get(&self.chain_id)
                .expect("!forwarder"),
            salt: None,
        })
    }

    fn type_hash() -> Result<[u8; 32], Self::Error> {
        Ok(keccak256(FORWARD_REQUEST_TYPE))
    }

    fn struct_hash(&self) -> Result<[u8; 32], Self::Error> {
        let encoded_request = abi::encode(&[
            Token::FixedBytes(Self::type_hash()?.to_vec()),
            Token::Uint(self.chain_id.into()),
            Token::Address(Address::from_str(&self.target).expect("!target")),
            Token::FixedBytes(keccak256(hex::decode(&self.data)?).to_vec()),
            Token::Address(Address::from_str(&self.fee_token).expect("!fee token")),
            Token::Uint(self.payment_type.into()),
            Token::Uint(self.max_fee.into()),
            Token::Uint(self.gas.into()),
            Token::Address(Address::from_str(&self.sponsor).expect("!sponsor")),
            Token::Uint(self.sponsor_chain_id.into()),
            Token::Uint(self.nonce.into()),
            Token::Bool(self.enforce_sponsor_nonce),
            Token::Bool(self.enforce_sponsor_nonce_ordering),
        ]);

        Ok(keccak256(encoded_request))
    }
}

impl UnfilledForwardRequest {
    /// Fill ForwardRequest with sponsor signature and return full request struct
    pub fn into_filled(self, sponsor_signature: Vec<u8>) -> ForwardRequest {
        let hex_sig = format!("0x{}", hex::encode(sponsor_signature));
        let hex_data = format!("0x{}", self.data);

        ForwardRequest {
            type_id: "ForwardRequest".to_owned(),
            chain_id: self.chain_id,
            target: self.target,
            data: hex_data,
            fee_token: self.fee_token,
            payment_type: self.payment_type,
            max_fee: self.max_fee.to_string(),
            gas: self.gas.to_string(),
            sponsor: self.sponsor,
            sponsor_chain_id: self.sponsor_chain_id,
            nonce: self.nonce,
            enforce_sponsor_nonce: self.enforce_sponsor_nonce,
            enforce_sponsor_nonce_ordering: self.enforce_sponsor_nonce_ordering,
            sponsor_signature: hex_sig,
        }
    }
}

#[cfg(test)]
mod test {
    use crate::UnfilledForwardRequest;
    use ethers::signers::LocalWallet;
    use ethers::signers::Signer;
    use ethers::types::transaction::eip712::Eip712;
    use once_cell::sync::Lazy;

    const DOMAIN_SEPARATOR: &str =
        "0x1b927f522830945610cf8f0521ef8b3f69352936e1b0920968dcad9cf1e30762";
    const DUMMY_SPONSOR_KEY: &str =
        "9cb3a530d61728e337290409d967db069f5219279f89e5ddb5ae4af76a8da5f4";
    const DUMMY_SPONSOR_ADDRESS: &str = "0x4e4f0d95bc1a4275b748a63221796080b1aa5c10";
    const SPONSOR_SIGNATURE: &str = "0x23c272c0cba2b897de0fd8fe87d419f0f273c82ef10917520b733da889688b1c6fec89412c6f121fccbc30ce89b20a3de2f405018f1ac1249b9ff705fdb62a521b";

    static REQUEST: Lazy<UnfilledForwardRequest> = Lazy::new(|| UnfilledForwardRequest {
        chain_id: 42,
        target: "0x61bBe925A5D646cE074369A6335e5095Ea7abB7A".to_owned(),
        data: "4b327067000000000000000000000000eeeeeeeeeeeeeeeeeeeeeeeeaeeeeeeeeeeeeeeeee"
            .to_owned(),
        fee_token: "0xEeeeeEeeeEeEeeEeEeEeeEEEeeeeEeeeeeeeEEeE".to_owned(),
        payment_type: 1,
        max_fee: 10000000000000000000,
        gas: 200000,
        sponsor: DUMMY_SPONSOR_ADDRESS.to_owned(),
        sponsor_chain_id: 42,
        nonce: 0,
        enforce_sponsor_nonce: false,
        enforce_sponsor_nonce_ordering: false,
    });

    #[test]
    fn it_computes_domain_separator() {
        let domain_separator = REQUEST.domain_separator().unwrap();

        assert_eq!(
            format!("0x{}", hex::encode(domain_separator)),
            DOMAIN_SEPARATOR,
        );
    }

    #[tokio::test]
    async fn it_computes_and_signs_digest() {
        let sponsor: LocalWallet = DUMMY_SPONSOR_KEY.parse().unwrap();
        assert_eq!(DUMMY_SPONSOR_ADDRESS, format!("{:#x}", sponsor.address()));

        let signature = sponsor.sign_typed_data(&*REQUEST).await.unwrap().to_vec();

        let hex_sig = format!("0x{}", hex::encode(signature));
        assert_eq!(SPONSOR_SIGNATURE, hex_sig);
    }
}
