// Licensed to the Apache Software Foundation (ASF) under one
// or more contributor license agreements.  See the NOTICE file
// distributed with this work for additional information
// regarding copyright ownership.  The ASF licenses this file
// to you under the Apache License, Version 2.0 (the
// "License"); you may not use this file except in compliance
// with the License.  You may obtain a copy of the License at
//
//   http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing,
// software distributed under the License is distributed on an
// "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied.  See the License for the
// specific language governing permissions and limitations
// under the License.

use crate::datatypes::Field;
use crate::{
    array::ArrayData, buffer::Buffer, datatypes::DataType, datatypes::UnionMode,
};

use super::{
    equal_range, equal_values, utils::child_logical_null_buffer_for_union,
    utils::equal_nulls,
};

// Checks if corresponding slots in two UnionArrays are same data types
fn equal_types(
    lhs_fields: &Vec<Field>,
    rhs_fields: &Vec<Field>,
    lhs_type_ids: &[i8],
    rhs_type_ids: &[i8],
) -> bool {
    let lhs_slots_types = lhs_type_ids
        .into_iter()
        .map(|type_id| lhs_fields.get(*type_id as usize).unwrap().data_type())
        .collect::<Vec<_>>();

    let rhs_slots_types = rhs_type_ids
        .into_iter()
        .map(|type_id| rhs_fields.get(*type_id as usize).unwrap().data_type())
        .collect::<Vec<_>>();

    lhs_slots_types
        .into_iter()
        .zip(rhs_slots_types.into_iter())
        .all(|(l, r)| l == r)
}

// Assumes lhs is "Dense" UnionArray and rhs is "Sparse" UnionArray
fn equal_dense_sparse(
    lhs: &ArrayData,
    rhs: &ArrayData,
    lhs_nulls: Option<&Buffer>,
    rhs_nulls: Option<&Buffer>,
    lhs_type_ids: &[i8],
    rhs_type_ids: &[i8],
    lhs_offsets: &[i32],
    rhs_start: usize,
) -> bool {
    lhs_type_ids
        .into_iter()
        .zip(rhs_type_ids.into_iter())
        .enumerate()
        .all(|(index, (l_type_id, r_type_id))| {
            let lhs_values = &lhs.child_data()[*l_type_id as usize];
            let rhs_values = &rhs.child_data()[*r_type_id as usize];

            let l_offset = lhs_offsets[index];

            let e_value = equal_range(
                lhs_values,
                rhs_values,
                None,
                None,
                l_offset as usize,
                rhs_start + index,
                1,
            );

            println!("e_value: {:?} ", e_value);
            e_value
        })
}

fn equal_dense(
    lhs: &ArrayData,
    rhs: &ArrayData,
    lhs_type_ids: &[i8],
    rhs_type_ids: &[i8],
    lhs_offsets: &[i32],
    rhs_offsets: &[i32],
) -> bool {
    let offsets = lhs_offsets.into_iter().zip(rhs_offsets.into_iter());

    lhs_type_ids
        .into_iter()
        .zip(rhs_type_ids.into_iter())
        .zip(offsets)
        .all(|((l_type_id, r_type_id), (l_offset, r_offset))| {
            let lhs_values = &lhs.child_data()[*l_type_id as usize];
            let rhs_values = &rhs.child_data()[*r_type_id as usize];

            equal_range(
                lhs_values,
                rhs_values,
                None,
                None,
                *l_offset as usize,
                *r_offset as usize,
                1,
            )
        })
}

fn equal_sparse(
    lhs: &ArrayData,
    rhs: &ArrayData,
    lhs_nulls: Option<&Buffer>,
    rhs_nulls: Option<&Buffer>,
    lhs_type_ids: &[i8],
    rhs_type_ids: &[i8],
    lhs_start: usize,
    rhs_start: usize,
) -> bool {
    lhs_type_ids
        .into_iter()
        .zip(rhs_type_ids.into_iter())
        .enumerate()
        .all(|(index, (l_type_id, r_type_id))| {
            let lhs_values = &lhs.child_data()[*l_type_id as usize];
            let rhs_values = &rhs.child_data()[*r_type_id as usize];

            // merge the null data
            let lhs_merged_nulls = child_logical_null_buffer_for_union(
                lhs, lhs_nulls, lhs_values, *l_type_id,
            );
            let rhs_merged_nulls = child_logical_null_buffer_for_union(
                rhs, rhs_nulls, rhs_values, *r_type_id,
            );

            equal_range(
                lhs_values,
                rhs_values,
                lhs_merged_nulls.as_ref(),
                rhs_merged_nulls.as_ref(),
                lhs_start + index,
                rhs_start + index,
                1,
            )
        })
}

pub(super) fn union_equal(
    lhs: &ArrayData,
    rhs: &ArrayData,
    lhs_nulls: Option<&Buffer>,
    rhs_nulls: Option<&Buffer>,
    lhs_start: usize,
    rhs_start: usize,
    len: usize,
) -> bool {
    let lhs_type_ids = lhs.buffer::<i8>(0);
    let rhs_type_ids = rhs.buffer::<i8>(0);

    match (lhs.data_type(), rhs.data_type()) {
        (
            DataType::Union(lhs_fields, UnionMode::Dense),
            DataType::Union(rhs_fields, UnionMode::Dense),
        ) => {
            let lhs_offsets = lhs.buffer::<i32>(1);
            let rhs_offsets = rhs.buffer::<i32>(1);

            let lhs_type_id_range = &lhs_type_ids[lhs_start..lhs_start + len];
            let rhs_type_id_range = &rhs_type_ids[rhs_start..rhs_start + len];

            let lhs_offsets_range = &lhs_offsets[lhs_start..lhs_start + len];
            let rhs_offsets_range = &rhs_offsets[rhs_start..rhs_start + len];

            // nullness is kept in the parent UnionArray, so we compare its nulls here
            equal_types(lhs_fields, rhs_fields, lhs_type_ids, rhs_type_ids)
                && equal_nulls(lhs, rhs, lhs_nulls, rhs_nulls, lhs_start, rhs_start, len)
                && equal_dense(
                    lhs,
                    rhs,
                    lhs_type_id_range,
                    rhs_type_id_range,
                    lhs_offsets_range,
                    rhs_offsets_range,
                )
        }
        (
            DataType::Union(lhs_fields, UnionMode::Sparse),
            DataType::Union(rhs_fields, UnionMode::Sparse),
        ) => {
            let lhs_type_id_range = &lhs_type_ids[lhs_start..lhs_start + len];
            let rhs_type_id_range = &rhs_type_ids[rhs_start..rhs_start + len];

            equal_types(lhs_fields, rhs_fields, lhs_type_ids, rhs_type_ids)
                && equal_sparse(
                    lhs,
                    rhs,
                    lhs_nulls,
                    rhs_nulls,
                    lhs_type_id_range,
                    rhs_type_id_range,
                    lhs_start,
                    rhs_start,
                )
        }
        (
            DataType::Union(lhs_fields, UnionMode::Dense),
            DataType::Union(rhs_fields, UnionMode::Sparse),
        ) => {
            let lhs_offsets = lhs.buffer::<i32>(1);

            let lhs_type_id_range = &lhs_type_ids[lhs_start..lhs_start + len];
            let rhs_type_id_range = &rhs_type_ids[rhs_start..rhs_start + len];

            let lhs_offsets_range = &lhs_offsets[lhs_start..lhs_start + len];

            equal_types(lhs_fields, rhs_fields, lhs_type_ids, rhs_type_ids)
                && equal_dense_sparse(
                    lhs,
                    rhs,
                    lhs_nulls,
                    rhs_nulls,
                    lhs_type_id_range,
                    rhs_type_id_range,
                    lhs_offsets_range,
                    rhs_start,
                )
        }
        (
            DataType::Union(lhs_fields, UnionMode::Sparse),
            DataType::Union(rhs_fields, UnionMode::Dense),
        ) => {
            let rhs_offsets = rhs.buffer::<i32>(1);

            let lhs_type_id_range = &lhs_type_ids[lhs_start..lhs_start + len];
            let rhs_type_id_range = &rhs_type_ids[rhs_start..rhs_start + len];

            let rhs_offsets_range = &rhs_offsets[rhs_start..rhs_start + len];

            equal_types(lhs_fields, rhs_fields, lhs_type_ids, rhs_type_ids)
                && equal_dense_sparse(
                    rhs,
                    lhs,
                    rhs_nulls,
                    lhs_nulls,
                    rhs_type_id_range,
                    lhs_type_id_range,
                    rhs_offsets_range,
                    lhs_start,
                )
        }
        _ => unreachable!(),
    }
}
