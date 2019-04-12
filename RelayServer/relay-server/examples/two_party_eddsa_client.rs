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
extern crate dict;


use std::env;
use std::io::{self, Read, Write};
use std::net::SocketAddr;
use std::{thread, time};
use std::cell::RefCell;
use std::vec::Vec;

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
    PeerIdentifier,
    MessagePayload,
};

// unique to our eddsa client
extern crate multi_party_ed25519;
extern crate curv;

use curv::elliptic::curves::ed25519::*;
use multi_party_ed25519::protocols::aggsig::{
    test_com, verify, KeyPair, Signature, EphemeralKey, KeyAgg
};
//use multi_party_ed25519::

use relay_server_common::common::*;


use dict::{ Dict, DictIface };
use std::collections::HashMap;

#[derive(Default, Debug)]
struct DataHolder{
    pub peer_id: Refcell<PeerIdentifier>,
    pub client_key: KeyPair,
    pub pks: HashMap<PeerIdentifier, Ed25519Point>,
    pub commitments: HashMap<PeerIdentifier, String>,
    pub r_s: HashMap<PeerIdentifier, String>,
    pub sigs: HashMap<PeerIdentifier, String>,
    pub capacity: u32,
    pub message: &'static[u8],
    pub agg_key: Option<KeyAgg>
}

impl DataHolder{
    pub fn new(capacity: u32, _message: &[u8]) -> DataHolder{
        DataHolder {
            client_key: None,
            pks: HashMap::new(),
            commitments: Vec::new(),
            r_s: HashMap::new(),
            sigs: HashMap::new(),
            capacity,
            message: _message,
            peer_id: Refcell::new(0),
            agg_key: None,
        }
    }

    pub fn zero_step(&mut self, peer_id:PeerIdentifier) -> Option<MessagePayload> {
        self.peer_id.replace(peer_id);
        self.client_key = KeyPair::create();
        let pk/*:Ed25519Point */= self.client_key.public_key;
        self.add_pk(peer_id, pk);


        let pk/*:Ed25519Point */= self.client_data.key.public_key;
        let pk_s = serde_json::to_string(&pk).expect("Failed in serialization");
        return Some(generate_pk_message_payload(&pk_s));
    }

    pub fn add_pk(&mut self, peer_id: PeerIdentifier, pk: Ed25519Point){
        self.pks.insert(peer_id, pk);
    }

    pub fn add_commitment(&mut self, peer_id: PeerIdentifier, commitment: String){self.commitments.insert(peer_id, commitment);/*TODO*/}
    pub fn add_r(&mut self, peer_id: PeerIdentifier, r:String){
        //let v = (r,blind_factor);
        self.r_s.insert(peer_id, r);
    }
    pub fn add_sig(&mut self, peer_id: PeerIdentifier, sig: String){
        self.sigs.insert(peer_id, sig);
    }

    pub fn do_step(&mut self) {
        if self.is_step_done() {
            // do the next step
            self.current_step += 1;
            match self.current_step {
                1 => self.client_data = self.data_holder.step_1(),
                2 => self.client_data = self.data_holder.step_2(),
                3 => self.client_data = self.data_holder.step_3(),
                _=>panic!("Unsupported step")
            }
        }
    }
    fn is_step_done(&mut self) -> bool {
        match self.current_step {
            0 => return self.is_done_step_0(),//from, payload), // in step 0 we move immediately to step 1
            1 => return self.is_done_step_1(),//from, payload),
            2 => return self.is_done_step_2(),//from, payload),
            3 => return self.is_done_step_3(),//from, payload),
            _=>panic!("Unsupported step")
        }
    }
}
impl DataHolder{

    /// data updaters for each step
    pub fn update_data_step_0(&mut self, from: PeerIdentifier, payload: MessagePayload){
        let payload_type = self.resolve_payload_type(&payload);
        match payload_type {
            MessagePayloadType ::PUBLIC_KEY(pk) => {
                let s_slice: &str = &pk[..];  // take a full slice of the string
                let _pk = serde_json::from_str( s_slice);
                match _pk {
                    Ok(_pk) => self.add_pk(from,_pk),
                    Err(e) => panic!("Could not serialize public key")
                }
            },
            _ => panic!("expected public key message")
        }
    }

    pub fn update_data_step_1(&mut self, from: PeerIdentifier, payload: MessagePayload){
        let payload_type = self.resolve_payload_type(&payload);
        match payload_type {
            MessagePayloadType ::COMMITMENT(t)  => {
                self.add_commitment(from, t);
            },
            _ => panic!("expected commitment message")
        }
    }

    pub fn update_data_step_2(&mut self, from: PeerIdentifier, payload: MessagePayload){
        let payload_type = self.resolve_payload_type(&payload);
        match payload_type {
            MessagePayloadType ::R_MESSAGE(r) => {
                self.add_r(from, r);
            },
            _ => panic!("expected R message")
        }
    }

    pub fn update_data_step_3(&mut self, from: PeerIdentifier, payload: MessagePayload){
        let payload_type = self.resolve_payload_type(&payload);
        match payload_type {
            MessagePayloadType ::SIGNATURE(s) => {
                self.add_signature(from, s);
            },
            _ => panic!("expected signature message")
        }
    }

    // TODO add a do step method to encapsulate
    /// validators to check if we finished a step
    pub fn is_done_step_0(&self) -> bool{
        self.pks.len() == self.capacity as usize
    }
    pub fn is_done_step_1(&self) -> bool{
        self.commitments.len() == self.capacity as usize
    }
    pub fn is_done_step_2(&self) -> bool{
        self.r_s.len() == self.capacity as usize
    }
    pub fn is_done_step_3(&self) -> bool{
        self.sigs.len() == self.capacity as usize
    }

    /// steps - in each step the client does a calculation on its
    /// data, and updates the data holder with the new data
    pub fn step_1(&mut self) -> Option<MessagePayload>{
        /// each peer computes its commitment to the ephemeral key
        /// (this implicitly means each party also calculates ephemeral key
        /// on this step)
        // round 1: send commitments to ephemeral public keys
        (ephemeral_key, sign_first_message, sign_second_message) =
            Signature::create_ephemeral_key_and_commit(&self.client_key, &self.message);

        let commitment = sign_first_message.commitment;
        // save the commitment
        match serde_json::to_string(commitment){
            Ok(json_string) =>{
                self.add_commitment(self.peer_id.clone().into_inner(), json_string.clone());
                let r = serde_json::to_string(&sign_second_message).expect("couldn't create R");
                //let blind_factor = serde_json::to_string(&sign_second_message.blind_factor).expect("Couldn't serialize blind factor");
                self.add_r(self.peer_id.clone().into_inner(), r);
                return Some(generate_commitment_message_payload((&json_string)));
            } ,
            Err(e) => panic!("Couldn't serialize commitment")
        }
    }
    pub fn step_2(&mut self) -> Option<MessagePayload>{
        /// step 2 - return the clients R. No extra calculations
        let r = self.r_s.get(self.peer_id.clone().into_inner()).unwrap_or_else(panic!("Didnt compute R"));
        //let msg_payload =
        return Some(generate_R_message_payload(&r));

    }
    pub fn step_3(&mut self) -> Option<MessagePayload>{
        /// step 3 - after validating all commitments,

        /// 1. compute APK
        /// 2. compute R' = sum(Ri)
        /// 3. sign message
        /// 4. generate (and return) signature message payload
        if !self.validate_commitments() {
            // commitments sent by others are not valid. exit
            panic!("Commitments not valid!")
        }
        self.aggregate_pks();
        self.compute_r_tot();
        let R_tot = self.R_tot.unwrap_or_else(panic!("Didn't compute R_tot!"));
        let apk = self.agg_key.unwrap_or_else(panic!("Didn't compute apk!"));


        let k = Signature::k(&R_tot, &party1_key_agg.apk, &message);
        let r = self.r_s.get(self.peer_id).unwrap_or_else(panic!("Client has No R ")).clone();
        let _r = serde_json::from_str(&r);
        let key = &self.client_key;
        // sign
        let s = Signature::partial_sign(&_r,&key,&k,&apk.hash,&R_tot);
        let sig_string = serde_json::to_string(&s).expect("failed to serialize signature");

        Some(generate_signature_message_payload(&sig_string))
    }

    /// check that the protocol is done
    /// and that this peer can finalize its calculations
    fn is_done(&mut self) -> bool {
        self.is_done_step_3()
    }

    /// Check if peer should finalize the session
    pub fn should_finalize(&mut self)->bool{
        self.is_done()
    }

    /// Does the final calculation of the protocol
    /// in this case:
    ///     collection all signatures
    ///     and verifying the message
    pub fn finalize(&mut self) -> Result<(),&'static str> {
        if !self.is_done(){
            return Err("not done")
        }
        // collect signatures
        let mut s: Vec<Signature> = Vec::new();
        for (peer_id, sig) in self.sigs {
            let signature = serde_json::from_str(&sig).expect("Could not serialize signature!");
            s.push(signature)
        }
        let signature = Signature::add_signature_parts(s);

        // verify message with signature
        let apk = self.agg_key.unwrap();
        if verify(&signature,&self.message, &apk.apk){
            Ok(())
        } else {
            Err("failed to verify message with aggregated signature")
        }

    }

    fn compute_r_tot(&mut self) {
        let mut Ri:Vec<GE> = Vec::new();
        for (peer_id, (r, blind)) in self.r_s {
            let _r = serde_json::from_str(r);
            Ri.push(_r.R.clone());
        }
        let r_tot= Signature::get_R_tot(Ri);
        self.R_tot = Some(r_tot);
    }

    fn aggregate_pks(&mut self) {
        let mut pks = Vec::with_capacity(self.capacity as usize);
        for (peer, pk) in self.pks {
            pks[peer - 1] = pk;
        }
        let agg_key= KeyPair::key_aggregation_n(&pks, self.peer_id - 1);
        self.agg_key = Some(agg_key);

    }

    fn validate_commitments(&mut self) -> bool{
        // iterate over all peer Rs
        for (peer_id, r) in self.r_s {
            // convert the json_string to a construct
            let _r = serde_json::from_str(&r).unwrap();

            // get the corresponding commitment
            let k = self.peer_id.clone().into_inner();
            let cmtmnt = self.commitments.get(k)
                .expect("peer didn't send commitment");
            let commitment = serde_json::from_str(cmtmnt).unwrap();
            // if we couldn't validate the commitment - failure
            if !test_com(
                &_r.R,
                &_r.blind_factor,
                commitment.unwrap_or_else(panic!("couldn't parse commitment json"))
            ) {
                return false;
            }
        }
        true
    }
}

impl DataHolder{
    fn resolve_payload_type(message: MessagePayload) -> MessagePayloadType  {
        let msg_payload = message.clone();

        let split_msg:Vec<&str> = msg_payload.split(RELAY_MESSAGE_DELIMITER).collect();
        let msg_prefix = split_msg[0];
        let msg_payload = String::from( split_msg[1].clone());
        match msg_prefix {

            pk_prefix if pk_prefix == String::from(PK_MESSAGE_PREFIX)=> {
                return MessagePayloadType ::PUBLIC_KEY(msg_payload);
            },
            cmtmnt if cmtmnt == String::from(COMMITMENT_MESSAGE_PREFIX) => {
                return MessagePayloadType ::COMMITMENT(msg_payload);
            },
            r if r == String::from(R_KEY_MESSAGE_PREFIX ) => {
                return MessagePayloadType::R_MESSAGE(msg_payload);

            },
            sig if sig == String::from(SIGNATURE_MESSAGE_PREFIX)=> {
                return MessagePayloadType ::SIGNATURE(msg_payload);
            },
            _ => panic!("Unknown relay message prefix")
        }
    }

}

struct ProtocolDataManager{
    pub peer_id: RefCell<PeerIdentifier>,
    pub capacity: u32,
    pub current_step: u32,
    pub data_holder: DataHolder, // will be filled when initializing, and on each new step
    pub client_data: Option<MessagePayload>, // new data calculated by this peer at the beginning of a step (that needs to be sent to other peers)
    pub new_client_data: bool,
}

impl ProtocolDataManager{
    pub fn new(capacity: u32, message:&[u8]) -> ProtocolDataManager{
        ProtocolDataManager {
            peer_id: Refcell::new(0),
            current_step: 0,
            capacity,
            data_holder: DataHolder::new(capacity, message),
            client_data: None,
            new_client_data: False,
            //message: message.clone(),
        }
    }

    /// set manager with the initial values that a local peer holds at the beginning of
    /// the protocol session
    /// return: first message
    pub fn initialize_data(&mut self, peer_id: PeerIdentifier) -> Option<MessagePayload>{
        self.peer_id.replace(peer_id);
        let zero_step_data = self.data_holder.zero_step(peer_id);
        self.client_data = zero_step_data;
        return self.client_data.clone();
    }

    pub fn update_data(&mut self, from: PeerIdentifier, payload: MessagePayload){
        // update data according to step
        match self.current_step {
            0 => self.data_holder.update_data_step_0(from, payload),
            1 => self.data_holder.update_data_step_1(from, payload),
            2 => self.data_holder.update_data_step_2(from, payload),
            3 => self.data_holder.update_data_step_3(from, payload),
            _=>panic!("Unsupported step")
        }

    }


    /// Update the data with the message received from the server
    /// according to the step we are
    pub fn handle_new_data(&mut self, from: PeerIdentifier, payload: MessagePayload) {
        self.update_data(from, payload);
        self.data_holder.do_step();
    }

    /// Return the next data this peer needs
    /// to send to other peers
    pub fn get_next_message(&mut self) -> MessagePayload{
        let data = self.client_data;
        match data {
            Some(data) => {
                self.client_data = None;
                return data;
            },
            None => {
                let m = relay_server_common::common::EMPTY_MESSAGE_PAYLOAD.clone();
                return m;
            },
        }
    }
}


// ClientSession holds session data
#[derive(Default, Debug, Clone)]
struct ProtocolSession {
    pub registered: bool,
    pub protocol_id: ProtocolIdentifier,
    pub data_manager: ProtocolDataManager,
    pub last_message: Option<ClientMessage>,
    pub bc_dests: Vec<ProtocolIdentifier>,
    pub timeout: u32,
}


impl ProtocolSession {
    pub fn new(protocol_id:ProtocolIdentifier, capacity: u32, message: &[u8]) -> ProtocolSession {
        ProtocolSession {
            registered: false,
            protocol_id,
            last_message: None,
            bc_dests: (1..(capacity+1)).collect(),
            timeout: 5000,
            data_manager: ProtocolDataManager::new(capacity, message),
        }
    }

    pub fn set_bc_dests(&mut self){
        let index = self.peer_id.clone().into_inner() - 1;
        self.bc_dests.remove(index as usize);
    }

    pub fn handle_relay_message(&mut self, msg: ServerMessage) -> MessagePayload{
        // parse relay message
        // (if we got here this means we are registered and
        // the client sent the private key)

        // so at the first step we are expecting the pks from all other peers
        let relay_msg = msg.relay_message.unwrap();
        let from = relay_msg.from;
        let payload = msg.message;
        self.data_manager.handle_new_data(from, payload);
        let answer = self.data_manager.get_next_message();
        return answer; //TODO CHANGE THIS

    }

    pub fn generate_client_answer(&mut self, msg: ServerMessage) -> Option<ClientMessage> {
        let msg_type = resolve_server_msg_type(msg.clone());
        match msg_type {
            ServerMessageType::Response =>{
                let next =self.handle_server_response(&msg);
                match next {
                    Ok(next_msg) => return Some(next_msg),
                    Err => panic!("Error in handle_server_response"),
                }
            },
            ServerMessageType::RelayMessage => {
                println!("Got new relay message");
                println!("{:?}", msg);
                let next = self.handle_relay_message(msg.clone());
                match next {
                    Ok(next_msg) => return Some(next_msg),
                    Err => panic!("Error in handle_relay_message"),
                }
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
    }

    fn handle_register_response(&mut self, peer_id: PeerIdentifier) ->Result<ClientMessage, ()>{
        println!("Peer identifier: {}",peer_id);
        // Set the session parameters
        session.peer_id.replace(peer_id);
        session.set_bc_dests();


        let message =  self.data_manager.initialize(peer_id);

        // create relay message
        let mut client_message = ClientMessage::new();
        let mut relay_message = RelayMessage::new(self.peer_id.clone().into_inner(), self.protocol_id.clone());
        let mut to: Vec<u32> = self.bc_dests.clone();

        // wait a little so we can spawn the other clients
        let wait_time = time::Duration::from_millis(self.timeout as u64);
        thread::sleep(wait_time);

        relay_message.set_message_params(0, to, &message);
        client_message.relay_message = Some(relay_message.clone());
        return Ok(client_message);
    }

    fn handle_error_response(&mut self, err_msg: &str) -> Result<ClientMessage, &'static str>{
        match  err_msg{
            resp if resp == String::from(NOT_YOUR_TURN) => {
                println!("not my turn");
                // wait
                let wait_time = time::Duration::from_millis(self.timeout as u64);
                thread::sleep(wait_time);
                println!("sending again");
                let msg = self.last_message.clone().unwrap();
                //TODO handle None
                return Ok(msg)
            },
            _ => {return Err(err_msg)}
        }
    }

    fn handle_server_response(&mut self, msg: &ServerMessage) -> Result<ClientMessage, &'static str>{
        let server_response = msg.response.unwrap();
        match server_response
            {
                ServerResponse::Register(peer_id) => {
                    let client_message = self.handle_register_response(peer_id);
                    match client_message{
                        Ok(_msg) => return Ok(_msg),
                        Err(e) => return Ok(ClientMessage::new()),
                    }
                },
                ServerResponse::ErrorResponse(err_msg) => {
                    println!("got error response");
                    let msg = self.handle_error_reponse(err_msg);
                    match msg {
                        Ok(_msg) => return Ok(_msg),
                        Err() => return Ok(ClientMessage::new()),
                    }
                },
                ServerResponse::GeneralResponse(msg) => {
                    unimplemented!()
                },
                ServerResponse::NoResponse => {
                    unimplemented!()
                },
                _ => panic!("failed to register")
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

pub fn resolve_server_msg_type(msg: ServerMessage) -> ServerMessageType{
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



enum MessagePayloadType {
    /// Types of expected relay messages
    /// for step 0 we expect PUBLIC_KEY_MESSAGE
    /// for step 1 we expect COMMITMENT
    /// for step 2 we expect R_MESSAGE
    /// for step 3 we expect SIGNATURE

    PUBLIC_KEY(String), //  Serialized key
    COMMITMENT(String), //  Commitment
    R_MESSAGE(String),  //  (R,blind) of the peer
    SIGNATURE(String),  //  S_j
}



fn main() {
    // message for signing
    let message: [u8; 4] = [79,77,69,82];


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


    let mut session = ProtocolSession::new(PROTOCOL_IDENTIFIER_ARG, PROTOCOL_CAPACITY_ARG, &message);
    let client = _tcp.and_then(|stream| {
        println!("sending register message");
        let framed_stream = stream.framed(ClientToServerCodec::new());

        // prepare register message
        let mut msg = ClientMessage::new();
        let register_msg = msg.register(session.protocol_id.clone(), session.capacity.clone());

        // send register message to server
        let send_ = framed_stream.send(msg);
        send_.and_then(|stream| {
            let (tx, rx) = stream.split();
            let client = rx.and_then(|msg| {
                println!("Got message from server: {:?}", msg);
                let result = session.generate_client_answer(msg);
                match result {
                    Some(msg) => return Ok(msg),
                    None => return Ok(ClientMessage::new()),
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

