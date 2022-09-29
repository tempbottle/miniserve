use actix_web::{dev::ServiceRequest, HttpMessage};
use actix_web_httpauth::extractors::basic::BasicAuth;
use sha2::{Digest, Sha256, Sha512};

use crate::errors::ContextualError;

#[derive(Clone, Debug)]
/// HTTP Basic authentication parameters
pub struct BasicAuthParams {
    pub username: String,
    pub password: String,
}

impl From<BasicAuth> for BasicAuthParams {
    fn from(auth: BasicAuth) -> Self {
        Self {
            username: auth.user_id().to_string(),
            password: auth.password().unwrap_or_default().to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// `password` field of `RequiredAuth`
pub enum RequiredAuthPassword {
    Plain(String),
    Sha256(Vec<u8>),
    Sha512(Vec<u8>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Authentication structure to match `BasicAuthParams` against
pub struct RequiredAuth {
    pub username: String,
    pub password: RequiredAuthPassword,
}

/// Return `true` if `basic_auth` is matches any of `required_auth`
pub fn match_auth(basic_auth: &BasicAuthParams, required_auth: &[RequiredAuth]) -> bool {
    required_auth
        .iter()
        .any(|RequiredAuth { username, password }| {
            basic_auth.username == *username && compare_password(&basic_auth.password, password)
        })
}

/// Return `true` if `basic_auth_pwd` meets `required_auth_pwd`'s requirement
pub fn compare_password(basic_auth_pwd: &str, required_auth_pwd: &RequiredAuthPassword) -> bool {
    match &required_auth_pwd {
        RequiredAuthPassword::Plain(required_password) => *basic_auth_pwd == *required_password,
        RequiredAuthPassword::Sha256(password_hash) => {
            compare_hash::<Sha256>(basic_auth_pwd, password_hash)
        }
        RequiredAuthPassword::Sha512(password_hash) => {
            compare_hash::<Sha512>(basic_auth_pwd, password_hash)
        }
    }
}

/// Return `true` if hashing of `password` by `T` algorithm equals to `hash`
pub fn compare_hash<T: Digest>(password: &str, hash: &[u8]) -> bool {
    get_hash::<T>(password) == hash
}

/// Get hash of a `text`
pub fn get_hash<T: Digest>(text: &str) -> Vec<u8> {
    let mut hasher = T::new();
    hasher.update(text);
    hasher.finalize().to_vec()
}

pub struct CurrentUser {
    pub name: String,
}

pub async fn handle_auth(
    req: ServiceRequest,
    cred: BasicAuth,
) -> actix_web::Result<ServiceRequest, (actix_web::Error, ServiceRequest)> {
    let required_auth = &req.app_data::<crate::MiniserveConfig>().unwrap().auth;

    req.extensions_mut().insert(CurrentUser {
        name: cred.user_id().to_string(),
    });

    if match_auth(&cred.into(), required_auth) {
        Ok(req)
    } else {
        Err((ContextualError::InvalidHttpCredentials.into(), req))
    }
}

#[rustfmt::skip]
#[cfg(test)]
mod tests {
    use super::*;
    use rstest::{rstest, fixture};
    use pretty_assertions::assert_eq;

    /// Return a hashing function corresponds to given name
    fn get_hash_func(name: &str) -> impl FnOnce(&str) -> Vec<u8> {
        match name {
            "sha256" => get_hash::<Sha256>,
            "sha512" => get_hash::<Sha512>,
            _ => panic!("Invalid hash method"),
        }
    }

    #[rstest(
        password, hash_method, hash,
        case("abc", "sha256", "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"),
        case("abc", "sha512", "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"),
    )]
    fn test_get_hash(password: &str, hash_method: &str, hash: &str) {
        let hash_func = get_hash_func(hash_method);
        let expected = hex::decode(hash).expect("Provided hash is not a valid hex code");
        let received = hash_func(&password.to_owned());
        assert_eq!(received, expected);
    }

    /// Helper function that creates a `RequiredAuth` structure and encrypt `password` if necessary
    fn create_required_auth(username: &str, password: &str, encrypt: &str) -> RequiredAuth {
        use RequiredAuthPassword::*;

        let password = match encrypt {
            "plain" => Plain(password.to_owned()),
            "sha256" => Sha256(get_hash::<sha2::Sha256>(&password.to_owned())),
            "sha512" => Sha512(get_hash::<sha2::Sha512>(&password.to_owned())),
            _ => panic!("Unknown encryption type"),
        };

        RequiredAuth {
            username: username.to_owned(),
            password,
        }
    }

    #[rstest(
        should_pass, param_username, param_password, required_username, required_password, encrypt,
        case(true, "obi", "hello there", "obi", "hello there", "plain"),
        case(false, "obi", "hello there", "obi", "hi!", "plain"),
        case(true, "obi", "hello there", "obi", "hello there", "sha256"),
        case(false, "obi", "hello there", "obi", "hi!", "sha256"),
        case(true, "obi", "hello there", "obi", "hello there", "sha512"),
        case(false, "obi", "hello there", "obi", "hi!", "sha512")
    )]
    fn test_single_auth(
        should_pass: bool,
        param_username: &str,
        param_password: &str,
        required_username: &str,
        required_password: &str,
        encrypt: &str,
    ) {
        assert_eq!(
            match_auth(
                &BasicAuthParams {
                    username: param_username.to_owned(),
                    password: param_password.to_owned(),
                },
                &[create_required_auth(required_username, required_password, encrypt)],
            ),
            should_pass,
        )
    }

    /// Helper function that creates a sample of multiple accounts
    #[fixture]
    fn account_sample() -> Vec<RequiredAuth> {
        [
            ("usr0", "pwd0", "plain"),
            ("usr1", "pwd1", "plain"),
            ("usr2", "pwd2", "sha256"),
            ("usr3", "pwd3", "sha256"),
            ("usr4", "pwd4", "sha512"),
            ("usr5", "pwd5", "sha512"),
        ]
            .iter()
            .map(|(username, password, encrypt)| create_required_auth(username, password, encrypt))
            .collect()
    }

    #[rstest(
        username, password,
        case("usr0", "pwd0"),
        case("usr1", "pwd1"),
        case("usr2", "pwd2"),
        case("usr3", "pwd3"),
        case("usr4", "pwd4"),
        case("usr5", "pwd5"),
    )]
    fn test_multiple_auth_pass(
        account_sample: Vec<RequiredAuth>,
        username: &str,
        password: &str,
    ) {
        assert!(match_auth(
            &BasicAuthParams {
                username: username.to_owned(),
                password: password.to_owned(),
            },
            &account_sample,
        ));
    }

    #[rstest]
    fn test_multiple_auth_wrong_username(account_sample: Vec<RequiredAuth>) {
        assert_eq!(match_auth(
            &BasicAuthParams {
                username: "unregistered user".to_owned(),
                password: "pwd0".to_owned(),
            },
            &account_sample,
        ), false);
    }

    #[rstest(
        username, password,
        case("usr0", "pwd5"),
        case("usr1", "pwd4"),
        case("usr2", "pwd3"),
        case("usr3", "pwd2"),
        case("usr4", "pwd1"),
        case("usr5", "pwd0"),
    )]
    fn test_multiple_auth_wrong_password(
        account_sample: Vec<RequiredAuth>,
        username: &str,
        password: &str,
    ) {
        assert_eq!(match_auth(
            &BasicAuthParams {
                username: username.to_owned(),
                password: password.to_owned(),
            },
            &account_sample,
        ), false);
    }
}
