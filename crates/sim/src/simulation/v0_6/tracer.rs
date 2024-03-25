// This file is part of Rundler. //
// Rundler is free software: you can redistribute it and/or modify it under the
// terms of the GNU Lesser General Public License as published by the Free Software
// Foundation, either version 3 of the License, or (at your option) any later version.
//
// Rundler is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY;
// without even the implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
// See the GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License along with Rundler.
// If not, see https://www.gnu.org/licenses/.

use std::{collections::HashMap, convert::TryFrom, fmt::Debug, sync::Arc};

use anyhow::{bail, Context};
use async_trait::async_trait;
use ethers::types::{
    Address, BlockId, GethDebugTracerType, GethDebugTracingCallOptions, GethDebugTracingOptions,
    GethTrace, Opcode,
};
#[cfg(test)]
use mockall::automock;
use rundler_provider::{Provider, SimulationProvider};
use rundler_types::v0_6::UserOperation;
use serde::{Deserialize, Serialize};

use crate::{
    simulation::{AccessInfo, AssociatedSlotsByAddress},
    ExpectedStorage,
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimulationTracerOutput {
    pub(crate) phases: Vec<Phase>,
    pub(crate) revert_data: Option<String>,
    pub(crate) accessed_contract_addresses: Vec<Address>,
    pub(crate) associated_slots_by_address: AssociatedSlotsByAddress,
    pub(crate) factory_called_create2_twice: bool,
    pub(crate) expected_storage: ExpectedStorage,
}

impl TryFrom<GethTrace> for SimulationTracerOutput {
    type Error = anyhow::Error;
    fn try_from(trace: GethTrace) -> Result<Self, Self::Error> {
        match trace {
            GethTrace::Unknown(value) => Ok(SimulationTracerOutput::deserialize(&value)?),
            GethTrace::Known(_) => {
                bail!("Failed to deserialize simulation trace")
            }
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Phase {
    pub(crate) forbidden_opcodes_used: Vec<String>,
    pub(crate) forbidden_precompiles_used: Vec<String>,
    pub(crate) storage_accesses: HashMap<Address, AccessInfo>,
    pub(crate) called_banned_entry_point_method: bool,
    pub(crate) addresses_calling_with_value: Vec<Address>,
    pub(crate) called_non_entry_point_with_value: bool,
    pub(crate) ran_out_of_gas: bool,
    pub(crate) undeployed_contract_accesses: Vec<Address>,
    pub(crate) ext_code_access_info: HashMap<Address, Opcode>,
}

/// Trait for tracing the simulation of a user operation.
#[cfg_attr(test, automock)]
#[async_trait]
pub trait SimulateValidationTracer: Send + Sync + 'static {
    /// Traces the simulation of a user operation.
    async fn trace_simulate_validation(
        &self,
        op: UserOperation,
        block_id: BlockId,
        max_validation_gas: u64,
    ) -> anyhow::Result<SimulationTracerOutput>;
}

/// Tracer implementation for the bundler's custom tracer.
#[derive(Debug)]
pub struct SimulateValidationTracerImpl<P, E> {
    provider: Arc<P>,
    entry_point: E,
}

/// Runs the bundler's custom tracer on the entry point's `simulateValidation`
/// method for the provided user operation.

#[async_trait]
impl<P, E> SimulateValidationTracer for SimulateValidationTracerImpl<P, E>
where
    P: Provider,
    E: SimulationProvider<UO = UserOperation>,
{
    async fn trace_simulate_validation(
        &self,
        op: UserOperation,
        block_id: BlockId,
        max_validation_gas: u64,
    ) -> anyhow::Result<SimulationTracerOutput> {
        let (tx, state_override) = self
            .entry_point
            .get_tracer_simulate_validation_call(op, max_validation_gas);

        SimulationTracerOutput::try_from(
            self.provider
                .debug_trace_call(
                    tx,
                    Some(block_id),
                    GethDebugTracingCallOptions {
                        tracing_options: GethDebugTracingOptions {
                            tracer: Some(GethDebugTracerType::JsTracer(
                                validation_tracer_js().to_string(),
                            )),
                            ..Default::default()
                        },
                        state_overrides: Some(state_override),
                    },
                )
                .await?,
        )
    }
}

impl<P, E> SimulateValidationTracerImpl<P, E> {
    /// Creates a new instance of the bundler's custom tracer.
    pub fn new(provider: Arc<P>, entry_point: E) -> Self {
        Self {
            provider,
            entry_point,
        }
    }
}

fn validation_tracer_js() -> &'static str {
    include_str!("../../../tracer/dist/validationTracer.js").trim_end_matches(";export{};")
}

pub(crate) fn parse_combined_tracer_str<A, B>(combined: &str) -> anyhow::Result<(A, B)>
where
    A: std::str::FromStr,
    B: std::str::FromStr,
    <A as std::str::FromStr>::Err: std::error::Error + Send + Sync + 'static,
    <B as std::str::FromStr>::Err: std::error::Error + Send + Sync + 'static,
{
    let (a, b) = combined
        .split_once(':')
        .context("tracer combined should contain two parts")?;
    Ok((a.parse()?, b.parse()?))
}
