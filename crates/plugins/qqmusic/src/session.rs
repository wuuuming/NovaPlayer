//! Retain refresh results while the host keeps sending the original opaque token.
use crate::api::{Client, Credential};
use kit::json;

#[derive(Default)]
pub struct Session {
    source_token: Option<String>,
    credential: Option<Credential>,
}

impl Session {
    pub fn clear(&mut self) {
        self.source_token = None;
        self.credential = None;
    }

    pub fn credential(&mut self, client: &Client, token: Option<&str>) -> Result<Option<Credential>, String> {
        self.get_with(token, |c| client.refresh(c))
    }

    fn get_with(
        &mut self,
        token: Option<&str>,
        refresh: impl FnOnce(&Credential) -> Option<Credential>,
    ) -> Result<Option<Credential>, String> {
        let Some(token) = token else {
            self.clear();
            return Ok(None);
        };
        if self.source_token.as_deref() != Some(token) {
            self.clear();
            let c = Credential::from_json(
                &json::parse(token).map_err(|_| kit::pipe::tr("凭据损坏", "The credentials are damaged"))?,
            );
            if !c.usable() {
                return Err(kit::pipe::tr("凭据损坏", "The credentials are damaged").into());
            }
            self.source_token = Some(token.to_owned());
            self.credential = Some(c);
        }
        let c = self.credential.as_ref().ok_or(kit::pipe::tr("凭据损坏", "The credentials are damaged"))?;
        if c.expired() {
            let new = refresh(c)
                .filter(|c| c.usable() && !c.expired())
                .ok_or(kit::pipe::tr("登录已过期，请重新登录", "The sign-in has expired; connect again"))?;
            self.credential = Some(new);
        }
        Ok(self.credential.clone())
    }
}
