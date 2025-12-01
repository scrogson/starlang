//! Integration tests for dream-macros.

use dream_macros::{dream_process, GenServerImpl};

#[derive(GenServerImpl)]
struct TestServer {
    counter: i64,
}

#[test]
fn test_gen_server_impl_derive() {
    assert_eq!(TestServer::type_name(), "TestServer");
}

#[dream_process]
async fn test_process() {
    // This just needs to compile
}

#[test]
fn test_dream_process_attribute() {
    // The attribute should not change the function signature
    // This test passes if it compiles
}
