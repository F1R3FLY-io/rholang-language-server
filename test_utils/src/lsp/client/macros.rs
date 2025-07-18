#[macro_export]
macro_rules! with_lsp_client {
    ($test_name:ident, $comm_type:expr, $callback:expr) => {
        #[tokio::test(flavor = "multi_thread")]
        async fn $test_name() {
            $crate::lsp::client::init_logger().expect("Failed to initialize logger");
            let (event_sender, event_receiver) = std::sync::mpsc::channel::<$crate::lsp::events::LspEvent>();

            match $crate::lsp::client::LspClient::start(
                String::from("rholang"),
                env!("CARGO_BIN_EXE_rholang-language-server").to_string(),
                $comm_type,
                event_sender,
            ).await {
                Ok(client) => {
                    let client = std::sync::Arc::new(client);
                    let event_thread = {
                        let client = std::sync::Arc::clone(&client);
                        tokio::task::spawn_blocking(move || {
                            for event in event_receiver {
                                match event {
                                    $crate::lsp::events::LspEvent::FileOpened { .. } => {
                                        client.handle_lsp_document_event(event)
                                    }
                                    $crate::lsp::events::LspEvent::TextChanged { .. } => {
                                        client.handle_lsp_document_event(event)
                                    }
                                    $crate::lsp::events::LspEvent::Exit => break,
                                    _ => {},
                                }
                            }
                        })
                    };

                    let result = client.initialize();
                    assert!(result.is_ok(), "Initialize failed: {}", result.unwrap_err());
                    let result = client.initialized();
                    assert!(result.is_ok(), "Initialized failed: {}", result.unwrap_err());
                    $callback(&client);
                    let result = client.shutdown();
                    assert!(result.is_ok(), "Shutdown failed: {}", result.unwrap_err());
                    let result = client.exit();
                    assert!(result.is_ok(), "Exit failed: {}", result.unwrap_err());
                    let result = client.stop().await;
                    assert!(result.is_ok(), "Stop failed: {}", result.unwrap_err());
                    event_thread.await.expect("Failed to await event thread");
                }
                Err(e) => {
                    panic!("Failed to start client: {}", e);
                }
            }
        }
    };
}
