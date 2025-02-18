#! /usr/bin/python3

# This script will provision required keys for Nomad Agents. 
# If keys have already been provisioned, it will fetch their details. 
#
# Keys will be provisioned for each configured environment, currently staging and production.
#
# It requires the following dependencies: 
#   pip3 install boto3 tabulate asn1tools web3
#
# It requires AWS Credentials to be set in a way that is compatible with the AWS SDK: 
#   https://docs.aws.amazon.com/cli/latest/userguide/cli-configure-files.html
#
# The User must have access to the following APIs: 
#   kms:CreateAlias
#   kms:CreateKey
#   kms:DescribeKey
#   kms:GetPublicKey
#   kms:ListAliases

import boto3
import json 
import logging 
from tabulate import tabulate
import asn1tools
from web3.auto import w3

# Logging Config 
logging.basicConfig(level=logging.DEBUG)
logging.getLogger('botocore').setLevel(logging.CRITICAL)
logging.getLogger('urllib3').setLevel(logging.CRITICAL)

logger = logging.getLogger("kms_provisioner")
logger.setLevel(logging.DEBUG)

# Agent Keys
agent_keys = {
    "development": [
        "watcher-signer",
        "watcher-attestation",
        "updater-signer",
        "updater-attestation",
        "processor-signer",
        "relayer-signer",
        "kathy-signer"
    ],
    "production": [
        "watcher-signer",
        "watcher-attestation",
        "updater-signer",
        "updater-attestation",
        "processor-signer",
        "relayer-signer",
    ]
}

networks = {
    "production": [
        "ethereum",
        "moonbeam",
        "evmos",
        "milkomeda-c1",
        "xdai",
        "polygonPOS",
        "avalanche",
        "binanceSmartChain",
        "optimism",
        "harmony",
        "arbitrum",
        "fantom",
        "bobaL2",
        "cronos",
        "godwoken",
        "moonriver",
        "fuse",
        "gather"
    ],
    "development": [
        "rinkeby",
        "kovan",
        "goerli",
        "xdai",
        "evmostestnet",
        "neontestnet",
        "optimism-kovan",
        "arbitrum-rinkeby"    
    ]
}

# nAgentKeys * nEnvironments
environments = [
    #"development"
    "production"
]

# AWS Region where we should provison keys
region = "us-west-2"

def get_kms_public_key(key_id: str) -> bytes:
    client = boto3.client('kms', region_name=region)
    logger.info(f"Fetching Public key for {key_id}")
    response = client.get_public_key(
        KeyId=key_id
    )

    return response['PublicKey']

def get_all_kms_aliases() -> list:
    kms = boto3.client('kms', region_name=region)
    truncated = True
    aliases = list()
    kwargs = { 'Limit': 100 }
    while truncated:
        response = kms.list_aliases(**kwargs)
        aliases = aliases + response['Aliases']
        truncated = response['Truncated']
        kwargs['Marker'] = response['NextMarker'] if truncated else ""

    return aliases

def calc_eth_address(pub_key) -> str:
    SUBJECT_ASN = '''
    Key DEFINITIONS ::= BEGIN
    SubjectPublicKeyInfo  ::=  SEQUENCE  {
       algorithm         AlgorithmIdentifier,
       subjectPublicKey  BIT STRING
     }
    AlgorithmIdentifier  ::=  SEQUENCE  {
        algorithm   OBJECT IDENTIFIER,
        parameters  ANY DEFINED BY algorithm OPTIONAL
      }
    END
    '''

    key = asn1tools.compile_string(SUBJECT_ASN)
    key_decoded = key.decode('SubjectPublicKeyInfo', pub_key)

    pub_key_raw = key_decoded['subjectPublicKey'][0]
    pub_key = pub_key_raw[1:len(pub_key_raw)]

    # https://www.oreilly.com/library/view/mastering-ethereum/9781491971932/ch04.html
    hex_address = w3.keccak(bytes(pub_key)).hex()
    eth_address = '0x{}'.format(hex_address[-40:])

    eth_checksum_addr = w3.toChecksumAddress(eth_address)

    return eth_checksum_addr


def main():

    data_headers = ["Alias Name", "Region", "Key ID", "ARN", "Key Description", "Ethereum Address"]
    data_rows = []

    kms = boto3.client('kms', region_name=region)
    current_aliases = get_all_kms_aliases()

    logger.debug(f"Fetched {len(current_aliases)} aliases from KMS")
    logger.debug(json.dumps(current_aliases, indent=2, default=str))
    
    for environment in environments:
        for network in networks[environment]:
            for key in agent_keys[environment]:

                key_name = f"{environment}-{network}-{key}"
                alias_name = f"alias/{key_name}"

                existing_alias = next((alias for alias in current_aliases if alias["AliasName"] == alias_name), None)

                if existing_alias == None:
                    logger.info(f"No existing alias found for {key_name}, creating new key")

                    key_response = kms.create_key(
                        Description=f'{environment} {network} {key}',
                        KeyUsage='SIGN_VERIFY',
                        Origin='AWS_KMS',
                        BypassPolicyLockoutSafetyCheck=False,
                        CustomerMasterKeySpec="ECC_SECG_P256K1",
                        Tags=[
                            {
                                'TagKey': 'environment',
                                'TagValue': environment
                        },]
                    )

                    alias_response = kms.create_alias(
                        # The alias to create. Aliases must begin with 'alias/'.
                        AliasName=alias_name,
                        # The identifier of the CMK whose alias you are creating. You can use the key ID or the Amazon Resource Name (ARN) of the CMK.
                        TargetKeyId=key_response["KeyMetadata"]["KeyId"],
                    )

                    logger.debug(json.dumps(key_response, indent=2, default=str))
                    logger.debug(json.dumps(alias_response, indent=2, default=str))


                    key_id = key_response["KeyMetadata"]["KeyId"]
                    key_arn = key_response["KeyMetadata"]["Arn"]
                    key_description = key_response["KeyMetadata"]["Description"]
                else: 
                    logger.info(f"Existing alias for {key_name}, fetching key.")

                    key_response = kms.describe_key(
                        KeyId=existing_alias["TargetKeyId"],
                    )

                    key_id = key_response["KeyMetadata"]["KeyId"]
                    key_arn = key_response["KeyMetadata"]["Arn"]
                    key_description = key_response["KeyMetadata"]["Description"]
                    
                logger.debug(f"Key Id: {key_id}")
                logger.debug(f"Key Arn: {key_arn}")
                logger.debug(f"Key Description: {key_description}")

                # Get the Ethereum Address from the KMS CMK
                public_key = get_kms_public_key(key_id)
                ethereum_address = calc_eth_address(public_key)

                data_rows.append([f'alias/{key_name}', region, key_id, key_arn, key_description, ethereum_address])
        

    # Print out the results of the operation
    print(tabulate(data_rows, data_headers, tablefmt="fancy_grid"))

if __name__ == '__main__':
    main()
