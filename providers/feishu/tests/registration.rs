use lankide_provider_feishu::{
    ClientError, FeishuRegistrar, RegistrationPoll, Request, Response, Transport, TransportError,
};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
struct RegistrationTransport {
    responses: Arc<Mutex<VecDeque<Response>>>,
}

impl RegistrationTransport {
    fn new(responses: impl IntoIterator<Item = Response>) -> Self {
        Self {
            responses: Arc::new(Mutex::new(responses.into_iter().collect())),
        }
    }
}

impl Transport for RegistrationTransport {
    fn execute(&self, _: Request) -> Result<Response, TransportError> {
        self.responses
            .lock()
            .expect("registration responses")
            .pop_front()
            .ok_or_else(|| TransportError::Failed("missing registration response".into()))
    }
}

fn response(status: u16, body: &str) -> Response {
    Response {
        status,
        headers: Vec::new(),
        body: body.as_bytes().to_vec(),
    }
}

#[test]
fn poll_accepts_feishu_pending_states_returned_with_http_400() {
    for (body, expected) in [
        (r#"{"error":"authorization_pending"}"#, "pending"),
        (r#"{"error":"slow_down"}"#, "slow_down"),
        (r#"{"error":"access_denied"}"#, "denied"),
        (r#"{"error":"expired_token"}"#, "expired"),
    ] {
        let registrar = FeishuRegistrar::new(RegistrationTransport::new([response(400, body)]));
        let actual = registrar.poll("device-code").expect("known poll state");
        assert!(
            matches!(
                (&actual, expected),
                (RegistrationPoll::Pending { slow_down: false }, "pending")
                    | (RegistrationPoll::Pending { slow_down: true }, "slow_down")
                    | (RegistrationPoll::Denied, "denied")
                    | (RegistrationPoll::Expired, "expired")
            ),
            "unexpected poll state: {actual:?}"
        );
    }
}

#[test]
fn poll_rejects_http_error_without_a_feishu_poll_state() {
    let registrar = FeishuRegistrar::new(RegistrationTransport::new([response(503, "{}")]));
    assert!(matches!(
        registrar.poll("device-code"),
        Err(ClientError::HttpStatus { status: 503, .. })
    ));
}
