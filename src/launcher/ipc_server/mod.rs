// Copyright 2015 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under (1) the MaidSafe.net Commercial License,
// version 1.0 or later, or (2) The General Public License (GPL), version 3, depending on which
// licence you accepted on initial access to the Software (the "Licences").
//
// By contributing code to the SAFE Network Software, or to this project generally, you agree to be
// bound by the terms of the MaidSafe Contributor Agreement, version 1.0.  This, along with the
// Licenses can be found in the root directory of this project at LICENSE, COPYING and CONTRIBUTOR.
//
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied.
//
// Please review the Licences for the specific language governing permissions and limitations
// relating to use of the SAFE Network Software.

pub mod events;

pub type EventSenderToServer<EventSubset> = ::event_sender::EventSender<events::IpcServerEventCategory, EventSubset>;

mod misc;
mod ipc_session;

const IPC_SERVER_THREAD_NAME: &'static str = "IpcServerThread";
const IPC_LISTENER_THREAD_NAME: &'static str = "IpcListenerThread";
const LISTENER_THIRD_OCTATE_START: u8 = 0;
const LISTENER_FOURTH_OCTATE_START: u8 = 1;

pub struct IpcServer {
    client               : ::std::sync::Arc<::std::sync::Mutex<::safe_core::client::Client>>,
    temp_id              : u32,
    _raii_joiner         : ::safe_core::utility::RAIIThreadJoiner,
    session_event_tx     : ::std::sync::mpsc::Sender<events::IpcSessionEvent>,
    session_event_rx     : ::std::sync::mpsc::Receiver<events::IpcSessionEvent>,
    listener_event_rx    : ::std::sync::mpsc::Receiver<events::IpcListenerEvent>,
    external_event_rx    : ::std::sync::mpsc::Receiver<events::ExternalEvent>,
    event_catagory_tx    : ::std::sync::mpsc::Sender<events::IpcServerEventCategory>,
    listener_endpoint    : String,
    listener_stop_flag   : ::std::sync::Arc<::std::sync::atomic::AtomicBool>,
    verified_sessions    : ::std::collections::HashMap<::routing::NameType, misc::SessionInfo>,
    unverified_sessions  : ::std::collections::HashMap<u32, misc::SessionInfo>,
    pending_verifications: ::std::collections::HashMap<String, misc::AppInfo>,
}

impl IpcServer {
    pub fn new(client: ::std::sync::Arc<::std::sync::Mutex<::safe_core::client::Client>>) -> Result<(::safe_core::utility::RAIIThreadJoiner,
                                                                                                     EventSenderToServer<events::ExternalEvent>),
                                                                                                    ::errors::LauncherError> {
        let (session_event_tx, session_event_rx) = ::std::sync::mpsc::channel();
        let (listener_event_tx, listener_event_rx) = ::std::sync::mpsc::channel();
        let (external_event_tx, external_event_rx) = ::std::sync::mpsc::channel();
        let (event_catagory_tx, event_catagory_rx) = ::std::sync::mpsc::channel();

        let stop_flag = ::std::sync::Arc::new(::std::sync::atomic::AtomicBool::new(false));

        let listener_event_sender = EventSenderToServer::<events::IpcListenerEvent>
                                                       ::new(listener_event_tx,
                                                             events::IpcServerEventCategory::IpcListenerEvent,
                                                             event_catagory_tx.clone());

        let (joiner, endpoint) = try!(IpcServer::spawn_acceptor(listener_event_sender,
                                                                stop_flag.clone()));

        let ipc_server = IpcServer {
            client               : client,
            temp_id              : 0,
            _raii_joiner         : joiner,
            session_event_tx     : session_event_tx,
            session_event_rx     : session_event_rx,
            listener_event_rx    : listener_event_rx,
            external_event_rx    : external_event_rx,
            event_catagory_tx    : event_catagory_tx.clone(),
            listener_endpoint    : endpoint,
            listener_stop_flag   : stop_flag,
            verified_sessions    : ::std::collections::HashMap::new(),
            unverified_sessions  : ::std::collections::HashMap::new(),
            pending_verifications: ::std::collections::HashMap::new(),
        };

        let ipc_server_joiner = eval_result!(::std::thread::Builder::new().name(IPC_SERVER_THREAD_NAME.to_string())
                                                                          .spawn(move || {
            IpcServer::activate_ipc_server(ipc_server, event_catagory_rx);
            debug!("Exiting Thread {:?}", IPC_SERVER_THREAD_NAME);
        }));

        let external_event_sender = EventSenderToServer::<events::ExternalEvent>
                                                       ::new(external_event_tx,
                                                             events::IpcServerEventCategory::ExternalEvent,
                                                             event_catagory_tx);

        Ok((::safe_core::utility::RAIIThreadJoiner::new(ipc_server_joiner), external_event_sender))
    }

    fn activate_ipc_server(mut ipc_server: IpcServer, event_catagory_rx: ::std::sync::mpsc::Receiver<events::IpcServerEventCategory>) {
        for event_category in event_catagory_rx.iter() {
            match event_category {
                events::IpcServerEventCategory::IpcListenerEvent => {
                    if let Ok(listner_event) = ipc_server.listener_event_rx.try_recv() {
                        match listner_event {
                           events::IpcListenerEvent::IpcListenerAborted(error)   => ipc_server.on_ipc_listener_aborted(error),
                           events::IpcListenerEvent::SpawnIpcSession(tcp_stream) => ipc_server.on_spawn_ipc_session(tcp_stream),
                        }
                    }
                },
                events::IpcServerEventCategory::IpcSessionEvent => {
                    if let Ok(session_event) = ipc_server.session_event_rx.try_recv() {
                        match session_event {
                            events::IpcSessionEvent::VerifySession(temp_id, nonce) => ipc_server.on_verify_session(temp_id, nonce),
                            events::IpcSessionEvent::IpcSessionWriteFailed(app_id) => ipc_server.on_ipc_session_write_failed(app_id),
                        }
                    }
                },
                events::IpcServerEventCategory::ExternalEvent => {
                    if let Ok(external_event) = ipc_server.external_event_rx.try_recv() {
                        match external_event {
                            events::ExternalEvent::AppActivated(activation_detail) => ipc_server.on_app_activated(activation_detail),
                            events::ExternalEvent::ChangeSafeDriveAccess(app_id, is_allowed) => ipc_server.on_change_safe_drive_access(app_id, is_allowed),
                            events::ExternalEvent::GetListenerEndpoint(sender) => ipc_server.on_get_listener_endpoint(sender),
                            events::ExternalEvent::Terminate => break,
                        }
                    }
                },
            }
        }
    }

    fn on_spawn_ipc_session(&mut self, stream: ::std::net::TcpStream) {
        let event_sender = EventSenderToServer::<events::IpcSessionEvent>
                                              ::new(self.session_event_tx.clone(),
                                                    events::IpcServerEventCategory::IpcSessionEvent,
                                                    self.event_catagory_tx.clone());
        match ipc_session::IpcSession::new(event_sender,
                                           self.temp_id,
                                           stream) {
            Ok((raii_joiner, event_sender)) => {
                if let Some(_) = self.unverified_sessions.insert(self.temp_id,
                                                                 misc::SessionInfo::new(raii_joiner,
                                                                                        event_sender)) {
                    debug!("Unverified session existed even after all temporary ids are exhausted. Terminating session ...");
                }
            },
            Err(err) => debug!("IPC Session spawning failed for peer {:?}", err),
        }
        self.temp_id = self.temp_id.wrapping_add(1);
    }

    fn on_ipc_listener_aborted(&self, error_str: String) {
        ;
    }

    fn on_verify_session(&mut self, temp_id: u32, nonce: String) {
        match (self.unverified_sessions.remove(&temp_id), self.pending_verifications.remove(&nonce)) {
            (Some(session_info), Some(app_info)) => {
                let app_detail = Box::new(ipc_session::events::event_data::AppDetail {
                    client: self.client.clone(),
                    app_id: app_info.app_id.clone(),
                    safe_drive_access: app_info.safe_drive_access,
                });

                if session_info.event_sender.send(ipc_session::events::ExternalEvent::AppDetailReceived(app_detail)).is_err() {
                    debug!("Unable to communicate with the session via channel. Session will be terminated.");
                } else if let Some(_) = self.verified_sessions.insert(app_info.app_id, session_info) {
                    debug!("Detected an attempt by an app to connect twice. Previous instance will be terminated.");
                }
            },
            _ => debug!("Temp Id {:?} and/or Nonce {:?} invalid. Possible security breach - situation salvaged.",
                        temp_id, nonce),
        }
    }

    fn on_ipc_session_write_failed(&self, app_id: Option<::routing::NameType>) {
        ;
    }

    fn on_app_activated(&mut self, activation_detail: Box<events::event_data::ActivationDetail>) {
        let detail = *activation_detail;
        if let Some(info) = self.pending_verifications.insert(detail.nonce,
                                                              misc::AppInfo::new(detail.app_id,
                                                                                 detail.app_root_dir_key,
                                                                                 detail.safe_drive_access)) {
            // TODO(Spandan) handle this security hole.
            debug!("Same nonce was already given to an app pending verification. This is a security hole not fixed in this iteration.");
            debug!("Dropping the previous app information and re-assigning nonce to a new app");
        }
    }

    fn on_change_safe_drive_access(&self, app_id: ::routing::NameType, is_allowed: bool) {
        ;
    }

    fn on_get_listener_endpoint(&self, sender: ::std::sync::mpsc::Sender<String>) {
        if let Err(err) = sender.send(self.listener_endpoint.clone()) {
            debug!("Error Sending Endpoint: {:?}", err);
        }
    }

    fn spawn_acceptor(event_sender: EventSenderToServer<events::IpcListenerEvent>,
                      stop_flag   : ::std::sync::Arc<::std::sync::atomic::AtomicBool>) -> Result<(::safe_core::utility::RAIIThreadJoiner,
                                                                                                  String),
                                                                                                 ::errors::LauncherError> {
        let mut third_octate = LISTENER_THIRD_OCTATE_START;
        let mut fourth_octate = LISTENER_FOURTH_OCTATE_START;

        let ipc_listener;

        loop {
            let local_ip = ::std::net::Ipv4Addr::new(127, 0, third_octate, fourth_octate);
            let local_endpoint = (local_ip, 0);

            match ::std::net::TcpListener::bind(local_endpoint) {
                Ok(listener) => {
                    ipc_listener = listener;
                    break;
                },
                Err(err) => {
                    debug!("Failed binding IPC Server on 127.0.{}.{} with error {:?}. Trying net IP...",
                           third_octate, fourth_octate, err);

                    if fourth_octate == 255 {
                        if third_octate == 255 {
                            return Err(::errors::LauncherError::IpcListenerCouldNotBeBound)
                        } else {
                            third_octate += 1;
                            fourth_octate = LISTENER_FOURTH_OCTATE_START;
                        }
                    } else {
                        fourth_octate += 1;
                    }
                },
            }
        }

        let local_endpoint = format!("{}", eval_result!(ipc_listener.local_addr()));

        let joiner = eval_result!(::std::thread::Builder::new().name(IPC_LISTENER_THREAD_NAME.to_string())
                                                               .spawn(move || {
            IpcServer::handle_accept(ipc_listener,
                                     event_sender,
                                     stop_flag);

            debug!("Exiting Thread {:?}", IPC_LISTENER_THREAD_NAME);
        }));

        Ok((::safe_core::utility::RAIIThreadJoiner::new(joiner), local_endpoint))
    }

    fn handle_accept(ipc_listener: ::std::net::TcpListener,
                     event_sender: EventSenderToServer<events::IpcListenerEvent>,
                     stop_flag   : ::std::sync::Arc<::std::sync::atomic::AtomicBool>) {
        loop  {
            match ipc_listener.accept() {
                Ok((stream, _)) => {
                    if stop_flag.load(::std::sync::atomic::Ordering::SeqCst) {
                        break;
                    } else {
                        if let Err(_) = event_sender.send(events::IpcListenerEvent::SpawnIpcSession(stream)) {
                            break;
                        }
                    }
                },
                Err(accept_error) => {
                    debug!("IPC Listener aborted !!");
                    let _ = event_sender.send(events::IpcListenerEvent::IpcListenerAborted(format!("{:?}", accept_error)));
                    break;
                },
            }
        }
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        self.listener_stop_flag.store(true, ::std::sync::atomic::Ordering::SeqCst);
        if let Ok(stream) = ::std::net::TcpStream::connect(&self.listener_endpoint[..]) {
            if let Err(err) = stream.shutdown(::std::net::Shutdown::Both) {
                debug!("Error shutting down terminator stream: {:?}", err);
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::io::Read;

    #[test]
    fn spawn_and_shut_ipc_server() {
        let client = ::std
                     ::sync
                     ::Arc::new(::std
                                ::sync
                                ::Mutex::new(eval_result!(::safe_core::utility::test_utils::get_client())));

        let (_raii_joiner_0, event_sender) = eval_result!(IpcServer::new(client));

        let (tx, rx) = ::std::sync::mpsc::channel();
        eval_result!(event_sender.send(::launcher::ipc_server::events::ExternalEvent::GetListenerEndpoint(tx)));
        let listener_ep = eval_result!(rx.recv());

        let mut stream = eval_result!(::std::net::TcpStream::connect(&listener_ep[..]));

        let _raii_joiner_1 = ::safe_core
                             ::utility
                             ::RAIIThreadJoiner
                             ::new(eval_result!(::std
                                                ::thread
                                                ::Builder::new().name("ReaderThread".to_string()).spawn(move || {
            let mut buffer = [0; 5];
            assert_eq!(eval_result!(stream.read(&mut buffer)), 0);
        })));

        ::std::thread::sleep_ms(3000);
        // Terminate to exit this test - otherwise the raii_joiners will hang this test - this is
        // by design. So there is no way out but graceful termination which is what this entire
        // design strives for.
        eval_result!(event_sender.send(::launcher::ipc_server::events::ExternalEvent::Terminate));
    }

    #[derive(Debug)]
    struct HandshakeRequest {
        pub end_point: String,
        pub data: HandshakePayload,
    }

    #[derive(Debug)]
    struct HandshakePayload {
        pub launcher_string: String,
        pub nonce: [u8; 24],
        pub public_encryption_key: [u8; 32],
    }

    impl ::rustc_serialize::json::ToJson for HandshakePayload {
        fn to_json(&self) -> ::rustc_serialize::json::Json {
            use ::rustc_serialize::base64::ToBase64;

            let mut tree = ::std::collections::BTreeMap::new();
            let config = ::config::get_base64_config();
            let base64_nonce = (&self.nonce).to_base64(config);
            let base64_pub_encryption_key = (&self.public_encryption_key).to_base64(config);

            if tree.insert("launcher_string".to_string(), self.launcher_string.to_json()).is_some() {
                error!("Json Conversion error -- HandshakePayload -- launcher_string");
            }
            if tree.insert("nonce".to_string(), base64_nonce.to_json()).is_some() {
                error!("Json Conversion error -- HandshakePayload -- nonce");
            }
            if tree.insert("public_encryption_key".to_string(), base64_pub_encryption_key.to_json()).is_some() {
                error!("Json Conversion error -- HandshakePayload -- public_encryption_key");
            }

            ::rustc_serialize::json::Json::Object(tree)
        }
    }


    impl ::rustc_serialize::json::ToJson for HandshakeRequest {
        fn to_json(&self) -> ::rustc_serialize::json::Json {
            use ::rustc_serialize::base64::ToBase64;

            let mut tree = ::std::collections::BTreeMap::new();
            let config = ::config::get_base64_config();

            if tree.insert("end_point".to_string(), self.end_point.to_json()).is_some() {
                error!("Json Conversion error -- HandshakeRequest -- endpoint");
            }
            if tree.insert("data".to_string(), self.data.to_json()).is_some() {
                error!("Json Conversion error -- HandshakeRequest -- data");
            }

            ::rustc_serialize::json::Json::Object(tree)
        }
    }

    #[test]
    fn application_handshake() {
        use ::rustc_serialize::json::ToJson;

        let client = ::std
                     ::sync
                     ::Arc::new(::std
                                ::sync
                                ::Mutex::new(eval_result!(::safe_core::utility::test_utils::get_client())));

        let (_raii_joiner_0, event_sender) = eval_result!(IpcServer::new(client));

        let (tx, rx) = ::std::sync::mpsc::channel();
        eval_result!(event_sender.send(::launcher::ipc_server::events::ExternalEvent::GetListenerEndpoint(tx)));
        let listener_ep = eval_result!(rx.recv());

        let mut stream = eval_result!(::std::net::TcpStream::connect(&listener_ep[..]));

        let app_id = ::routing::NameType(eval_result!(::safe_core::utility::generate_random_array_u8_64()));
        let dir_id = ::routing::NameType(eval_result!(::safe_core::utility::generate_random_array_u8_64()));
        let directory_key = ::safe_nfs::metadata::directory_key::DirectoryKey::new(dir_id,
                                                                                   10u64,
                                                                                   false,
                                                                                   ::safe_nfs::AccessLevel::Private);
        let activation_details = ::launcher::ipc_server::events::event_data::ActivationDetail {
            nonce            : "mock_nonce_string".to_string(),
            app_id           : app_id,
            app_root_dir_key : directory_key,
            safe_drive_access: false,
        };
        let activate_event = ::launcher::ipc_server::events::ExternalEvent::AppActivated(Box::new(activation_details));
        event_sender.send(activate_event);

        let _raii_joiner_1 = ::safe_core
                             ::utility
                             ::RAIIThreadJoiner
                             ::new(eval_result!(::std
                                                ::thread
                                                ::Builder::new().name("TCPClientThread".to_string())
                                                                .spawn(move || {
                use rustc_serialize::base64::ToBase64;

                let mut ipc_stream = eval_result!(::launcher
                                                  ::ipc_server
                                                  ::ipc_session
                                                  ::stream
                                                  ::IpcStream::new(eval_result!(stream.try_clone())));
                let app_nonce = ::sodiumoxide::crypto::box_::gen_nonce();
                let (app_public_key, app_seccret_key) = ::sodiumoxide::crypto::box_::gen_keypair();
                let payload = HandshakePayload {
                    launcher_string      : "mock_nonce_string".to_string(),
                    nonce                : app_nonce.0,
                    public_encryption_key: app_public_key.0,
                };
                let request = HandshakeRequest {
                    end_point: "safe-api/v1.0/handshake/authenticate-app".to_string(),
                    data     : payload,
                };

                let json_obj = request.to_json();
                let serialised_data = eval_result!(::safe_core::utility::serialise(&json_obj.to_string()));

                ipc_stream.write(serialised_data);

                ::std::thread::sleep_ms(1000);
                let mut buffer = [0; 1024];
                assert_eq!(eval_result!(stream.read(&mut buffer)), 1024);
        })));
        ::std::thread::sleep_ms(3000);
        eval_result!(event_sender.send(::launcher::ipc_server::events::ExternalEvent::Terminate));
    }
}
