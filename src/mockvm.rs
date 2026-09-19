//! A test host that serves mocked external calls correctly.
//!
//! `stylus_test::TestVM` 0.10.9 records mocked calls per (address, calldata)
//! but hands back the return data of whichever mock was *registered last*
//! (`perform_mocked_*_call` never updates `state.return_data`, and the SDK
//! reads results through `read_return_data`). A contract that makes several
//! different reads in one method therefore sees the wrong bytes. `MockVM`
//! wraps `TestVM`, delegates everything else to it, and serves the return
//! data of the mock that actually matched. An unmocked call panics with the
//! target and selector so a missing mock can never pass as a revert.

use alloc::{vec::Vec, format};
use core::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use alloy_primitives::{Address, B256, U256};
use stylus_sdk::stylus_core::host::{
    AccountAccess, BlockAccess, CalldataAccess, ChainAccess, CryptographyAccess, Host,
    MemoryAccess, MessageAccess, MeteringAccess, RawLogAccess, StorageAccess, UnsafeCallAccess,
    UnsafeDeploymentAccess,
};
use stylus_sdk::testing::TestVM;

type Key = (Address, Vec<u8>);
type Outcome = Result<Vec<u8>, Vec<u8>>;

#[derive(Default)]
struct Calls {
    static_returns: HashMap<Key, Outcome>,
    call_returns: HashMap<Key, Outcome>,
    /// Return data of the most recent (static or regular) call, what the SDK reads back.
    last: Vec<u8>,
    /// Every call made, in order, for assertions on what the contract touched.
    log: Vec<Key>,
}

#[derive(Clone)]
pub struct MockVM {
    inner: TestVM,
    calls: Rc<RefCell<Calls>>,
}

impl MockVM {
    pub fn new() -> Self {
        MockVM {
            inner: TestVM::new(),
            calls: Rc::new(RefCell::new(Calls::default())),
        }
    }

    pub fn set_block_timestamp(&self, t: u64) {
        self.inner.set_block_timestamp(t);
    }

    pub fn set_tx_origin(&self, origin: Address) {
        self.inner.set_tx_origin(origin);
    }

    pub fn mock_static_call(&self, to: Address, data: Vec<u8>, ret: Outcome) {
        self.calls.borrow_mut().static_returns.insert((to, data), ret);
    }

    pub fn mock_call(&self, to: Address, data: Vec<u8>, ret: Outcome) {
        self.calls.borrow_mut().call_returns.insert((to, data), ret);
    }

    /// (target, calldata) of every external call so far.
    pub fn call_log(&self) -> Vec<Key> {
        self.calls.borrow().log.clone()
    }

    fn serve(&self, kind: &str, to: Address, data: &[u8], outs_len: &mut usize) -> u8 {
        let key = (to, data.to_vec());
        let mut calls = self.calls.borrow_mut();
        calls.log.push(key.clone());
        let table = if kind == "static" {
            &calls.static_returns
        } else {
            &calls.call_returns
        };
        let Some(outcome) = table.get(&key).cloned() else {
            let selector = data.get(..4).map(hex_of).unwrap_or_default();
            panic!("unmocked {kind} call to {to} selector 0x{selector}");
        };
        let (status, bytes) = match outcome {
            Ok(bytes) => (0, bytes),
            Err(bytes) => (1, bytes),
        };
        *outs_len = bytes.len();
        calls.last = bytes;
        status
    }
}

fn hex_of(bytes: &[u8]) -> alloc::string::String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

impl Host for MockVM {}

impl CryptographyAccess for MockVM {
    fn native_keccak256(&self, input: &[u8]) -> B256 {
        self.inner.native_keccak256(input)
    }
}

impl CalldataAccess for MockVM {
    fn read_args(&self, len: usize) -> Vec<u8> {
        self.inner.read_args(len)
    }

    fn read_return_data(&self, offset: usize, size: Option<usize>) -> Vec<u8> {
        let calls = self.calls.borrow();
        let data = &calls.last;
        let start = offset.min(data.len());
        let end = match size {
            Some(s) => (start + s).min(data.len()),
            None => data.len(),
        };
        data[start..end].to_vec()
    }

    fn return_data_size(&self) -> usize {
        self.calls.borrow().last.len()
    }

    fn write_result(&self, data: &[u8]) {
        self.inner.write_result(data)
    }
}

unsafe impl UnsafeDeploymentAccess for MockVM {
    unsafe fn create1(
        &self,
        code: *const u8,
        code_len: usize,
        endowment: *const u8,
        contract: *mut u8,
        revert_data_len: *mut usize,
    ) {
        self.inner
            .create1(code, code_len, endowment, contract, revert_data_len)
    }

    unsafe fn create2(
        &self,
        code: *const u8,
        code_len: usize,
        endowment: *const u8,
        salt: *const u8,
        contract: *mut u8,
        revert_data_len: *mut usize,
    ) {
        self.inner
            .create2(code, code_len, endowment, salt, contract, revert_data_len)
    }
}

impl StorageAccess for MockVM {
    fn storage_load_bytes32(&self, key: U256) -> B256 {
        self.inner.storage_load_bytes32(key)
    }

    unsafe fn storage_cache_bytes32(&self, key: U256, value: B256) {
        self.inner.storage_cache_bytes32(key, value)
    }

    fn flush_cache(&self, clear: bool) {
        self.inner.flush_cache(clear)
    }
}

unsafe impl UnsafeCallAccess for MockVM {
    unsafe fn call_contract(
        &self,
        to: *const u8,
        data: *const u8,
        data_len: usize,
        _value: *const u8,
        _gas: u64,
        outs_len: &mut usize,
    ) -> u8 {
        let to = Address::from_slice(core::slice::from_raw_parts(to, 20));
        let data = core::slice::from_raw_parts(data, data_len);
        self.serve("call", to, data, outs_len)
    }

    unsafe fn static_call_contract(
        &self,
        to: *const u8,
        data: *const u8,
        data_len: usize,
        _gas: u64,
        outs_len: &mut usize,
    ) -> u8 {
        let to = Address::from_slice(core::slice::from_raw_parts(to, 20));
        let data = core::slice::from_raw_parts(data, data_len);
        self.serve("static", to, data, outs_len)
    }

    unsafe fn delegate_call_contract(
        &self,
        to: *const u8,
        data: *const u8,
        data_len: usize,
        gas: u64,
        outs_len: &mut usize,
    ) -> u8 {
        self.inner
            .delegate_call_contract(to, data, data_len, gas, outs_len)
    }
}

impl BlockAccess for MockVM {
    fn block_basefee(&self) -> U256 {
        self.inner.block_basefee()
    }

    fn block_coinbase(&self) -> Address {
        self.inner.block_coinbase()
    }

    fn block_number(&self) -> u64 {
        self.inner.block_number()
    }

    fn block_timestamp(&self) -> u64 {
        self.inner.block_timestamp()
    }

    fn block_gas_limit(&self) -> u64 {
        self.inner.block_gas_limit()
    }
}

impl ChainAccess for MockVM {
    fn chain_id(&self) -> u64 {
        self.inner.chain_id()
    }
}

impl AccountAccess for MockVM {
    fn balance(&self, account: Address) -> U256 {
        self.inner.balance(account)
    }

    fn contract_address(&self) -> Address {
        self.inner.contract_address()
    }

    fn code(&self, account: Address) -> Vec<u8> {
        self.inner.code(account)
    }

    fn code_size(&self, account: Address) -> usize {
        self.inner.code_size(account)
    }

    fn code_hash(&self, account: Address) -> B256 {
        self.inner.code_hash(account)
    }
}

impl MemoryAccess for MockVM {
    fn pay_for_memory_grow(&self, pages: u16) {
        self.inner.pay_for_memory_grow(pages)
    }
}

impl MessageAccess for MockVM {
    fn msg_sender(&self) -> Address {
        self.inner.msg_sender()
    }

    fn msg_reentrant(&self) -> bool {
        self.inner.msg_reentrant()
    }

    fn msg_value(&self) -> U256 {
        self.inner.msg_value()
    }

    fn tx_origin(&self) -> Address {
        self.inner.tx_origin()
    }
}

impl MeteringAccess for MockVM {
    fn evm_gas_left(&self) -> u64 {
        self.inner.evm_gas_left()
    }

    fn evm_ink_left(&self) -> u64 {
        self.inner.evm_ink_left()
    }

    fn tx_gas_price(&self) -> U256 {
        self.inner.tx_gas_price()
    }

    fn tx_ink_price(&self) -> u32 {
        self.inner.tx_ink_price()
    }
}

impl RawLogAccess for MockVM {
    fn emit_log(&self, input: &[u8], num_topics: usize) {
        self.inner.emit_log(input, num_topics)
    }

    fn raw_log(&self, topics: &[B256], data: &[u8]) -> Result<(), &'static str> {
        self.inner.raw_log(topics, data)
    }
}
