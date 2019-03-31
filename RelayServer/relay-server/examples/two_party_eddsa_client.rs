#![feature(refcell_replace_swap)]
///
/// Implementation of a client that communicates with the relay server
/// this implememnataion is simplistic and used for POC and development and debugging of the
/// server
///
///
extern crate futures;
extern crate tokio_core;
extern crate relay_server_common;

use std::env;
use std::io::{self, Read, Write};
use std::net::SocketAddr;
use std::{thread, time};
use std::cell::RefCell;

use tokio_core::reactor::Core;
use tokio_core::net::TcpStream;
use tokio_core::io::Io;

use futures::{Stream, Sink, Future};
use futures::sync::mpsc;

use relay_server_common::{
    ClientToServerCodec,
    ClientMessage,
    ServerMessage,
    ServerResponse,
    RelayMessage,
    ProtocolIdentifier,
    PeerIdentifier
};

extern crate multi_party_ed25519;
extern crate curv;

use protocols::aggsig::{test_com, verify, KeyPair, Signature};

// ClientSession holds session data
#[derive(Default, Debug, Clone)]
struct ProtocolSession {
    pub registered: bool,
    pub peer_id: RefCell<PeerIdentifier>,
    pub protocol_id: ProtocolIdentifier,
    pub capacity: u32,
    pub next_message: Option<ClientMessage>,
}

impl ProtocolSession {
    pub fn new(protocol_id:ProtocolIdentifier, capacity: u32) -> ProtocolSession {
        ProtocolSession {
            registered: false,
            peer_id: Refcell::new(0),
            protocol_id,
            capacity,
            next_message: None,
        }
    }
}

#[derive(Debug)]
pub enum ServerMessageType { // TODO this is somewhat duplicate
    Response,
    Abort,
    RelayMessage,
    Undefined,
}

pub fn resolve_msg_type(msg: ServerMessage) -> ServerMessageType{
    if msg.response.is_some(){
        return ServerMessageType::Response;
    }
    if msg.relay_message.is_some(){
        return ServerMessageType::RelayMessage;
    }
    if msg.abort.is_some(){
        return ServerMessageType::Abort;
    }
    return ServerMessageType::Undefined;
}

pub enum MessageProcessResult {
    Message,
    NoMessage,
    Abort
}

fn main() {
    // TODO take these from ARGV
    let PROTOCOL_IDENTIFIER_ARG = 1;
    let PROTOCOL_CAPACITY_ARG = 2 as ProtocolIdentifier;

    let mut args = env::args().skip(1).collect::<Vec<_>>();
    // Parse what address we're going to co nnect to
    let addr = args.first().unwrap_or_else(|| {
        panic!("this program requires at least one argument")
    });

    let addr = addr.parse::<SocketAddr>().unwrap();

    // Create the event loop and initiate the connection to the remote server
    let mut core = Core::new().unwrap();
    let handle = core.handle();
    let _tcp = TcpStream::connect(&addr, &handle);


    let mut session = ProtocolSession::new(PROTOCOL_IDENTIFIER_ARG, PROTOCOL_CAPACITY_ARG);

    let client = _tcp.and_then(|stream| {
        println!("sending register message");

        let framed_stream = stream.framed(ClientToServerCodec::new());


        // prepare register message
        let mut msg = ClientMessage::new();
        let register_msg = msg.register(session.protocol_id.clone(), session.capacity.clone());

        let session = session.clone();
        // send register message to server
        framed_stream.send(msg)
            .and_then(|stream| {
                let (tx, rx) = stream.split();
                let client = rx.and_then(|msg| {
                    println!("Got message from server: {:?}", msg);
                    let msg_type = resolve_msg_type(msg.clone());
                    match msg_type {
                        ServerMessageType::Response =>{
                            let server_response = msg.response.unwrap();
                            match server_response {
                                ServerResponse::Register(peer_id) => {
                                    println!("Peer identifier: {}",peer_id);
                                    session.peer_id.replace(peer_id);

                                    // create a mock relay message
                                    let mut client_message= ClientMessage::new();
                                    let mut relay_message = RelayMessage::new(peer_id, protocol_id);
                                    let mut to: Vec<u32> = Vec::new();
                                    if peer_id == 2{
                                        to.push(1);
                                    } else {
                                        to.push(2);
                                    }

                                    // wait a little so we can spawn the second client
                                    let wait_time = time::Duration::from_millis(5000);
                                    thread::sleep(wait_time);

                                    relay_message.set_message_params(0, to, format!("Hi from {}", peer_id));
                                    client_message.relay_message = Some(relay_message.clone());
                                    //session.next_message = Some(client_message);
                                    return Ok(client_message);
                                },
                                ServerResponse::ErrorResponse(err_msg) => println!("got error response"),
                                _ => panic!("failed to register")
                            }
                        }
                        ServerMessageType::RelayMessage => {
                            println!("Got new relay message");
                            println!("{:?}", msg.relay_message.unwrap());
                            //Ok(MessageProcessResult::NoMessage)
                            Ok(ClientMessage::new())
                        },
                        ServerMessageType::Abort => {
                            println!("Got abort message");
                            //Ok(MessageProcessResult::NoMessage)
                            Ok(ClientMessage::new())
                        },
                        ServerMessageType::Undefined => {
                            Ok(ClientMessage::new())
                            //panic!("Got undefined message: {:?}",msg);
                        }
                    }
                }).forward(tx);
                client
            })
    })
        .map_err(|err| {
            // All tasks must have an `Error` type of `()`. This forces error
            // handling and helps avoid silencing failures.
            //
            // In our example, we are only going to log the error to STDOUT.
            println!("connection error = {:?}", err);
        });


    core.run(client);//.unwrap();

}