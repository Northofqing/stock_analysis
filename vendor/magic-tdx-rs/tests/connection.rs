use magic_tdx_rs::net::connection::TcpConnection;

fn connection_error(ip: &str, port: u16, timeout: f64) -> String {
    match TcpConnection::connect(ip, port, timeout) {
        Ok(mut connection) => {
            connection.close();
            panic!("connection unexpectedly succeeded")
        }
        Err(error) => error.to_string(),
    }
}

#[test]
fn connection_rejects_invalid_address_and_timeout_before_transport() {
    for timeout in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        let error = connection_error("127.0.0.1", 7709, timeout);
        assert!(error.contains("positive finite"));
    }

    let error = connection_error("not-an-ip", 7709, 0.01);
    assert!(error.contains("invalid TDX server address"));
}
