// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

use bytecode_source_map::source_map::ModuleSourceMap;
use failure::prelude::*;
use ir_to_bytecode::{
    compiler::compile_module,
    parser::{ast::Loc, parse_module},
};
use libra_types::account_address::AccountAddress;
use std::{fs, path::Path};
use vm::{access::ModuleAccess, file_format::CompiledModule};

pub fn do_compile_module<T: ModuleAccess>(
    source_path: &Path,
    address: AccountAddress,
    dependencies: &[T],
) -> (CompiledModule, ModuleSourceMap<Loc>) {
    let source = fs::read_to_string(source_path)
        .unwrap_or_else(|_| unrecoverable!("Unable to read file: {:?}", source_path));
    let parsed_module = parse_module(&source).unwrap();
    compile_module(address, parsed_module, dependencies).unwrap()
}
