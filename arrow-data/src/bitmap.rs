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

//! Defines [Bitmap] for tracking validity bitmaps

use arrow_buffer::bit_util;
use arrow_schema::ArrowError;
use std::mem;

use arrow_buffer::bit_chunk_iterator::UnalignedBitChunk;
use arrow_buffer::buffer::{buffer_bin_and, buffer_bin_or, Buffer};
use std::ops::{BitAnd, BitOr};

#[derive(Debug, Clone)]
/// Defines a bitmap, which is used to track which values in an Arrow
/// array are null.
///
/// This is called a "validity bitmap" in the Arrow documentation.
pub struct Bitmap {
    pub(crate) bits: Buffer,

    /// The offset into the bitmap.
    offset: usize,

    /// Bit length of the bitmap.
    length: usize,
}

impl Bitmap {
    pub fn new(num_bits: usize) -> Self {
        let num_bytes = bit_util::ceil(num_bits, 8);
        let len = bit_util::round_upto_multiple_of_64(num_bytes);
        Bitmap {
            bits: Buffer::from(&vec![0xFF; len]),
            offset: 0,
            length: num_bits,
        }
    }

    pub fn new_from_buffer(buf: Buffer, offset: usize, length: usize) -> Self {
        assert!(
            offset + length <= buf.len() * 8,
            "the offset + length of the new Bitmap cannot exceed the bit length of the Buffer"
        );
        Bitmap {
            bits: buf,
            offset,
            length,
        }
    }

    pub fn offset(&self) -> usize {
        self.offset
    }

    /// Return the length of this Bitmap in bits (not bytes)
    pub fn bit_len(&self) -> usize {
        self.length
    }

    pub fn is_empty(&self) -> bool {
        self.bits.is_empty()
    }

    pub fn is_set(&self, i: usize) -> bool {
        assert!(i < self.length);
        unsafe { bit_util::get_bit_raw(self.bits.as_ptr().add(self.offset), i) }
    }

    #[deprecated(note = "Direct access to bitmap's buffer is deprecated.")]
    pub fn buffer(&self) -> &Buffer {
        &self.bits
    }

    #[deprecated(note = "Direct access to bitmap's buffer is deprecated.")]
    pub fn buffer_ref(&self) -> &Buffer {
        assert!(self.offset == 0);
        &self.bits
    }

    pub fn into_buffer(self) -> Buffer {
        self.bits
    }

    /// Returns the total number of bytes of memory occupied by the
    /// buffers owned by this [Bitmap].
    ///
    /// If multiple [`Bitmap`]s refer to the same underlying
    /// [`Buffer`] they will both report the same size.
    pub fn get_buffer_memory_size(&self) -> usize {
        self.bits.capacity()
    }

    /// Returns the total number of bytes of memory occupied
    /// physically by this [Bitmap] and its [`Buffer`]s.
    ///
    /// Equivalent to: `size_of_val(self)` + [`Self::get_buffer_memory_size`]
    pub fn get_array_memory_size(&self) -> usize {
        self.bits.capacity() + mem::size_of_val(self)
    }

    /// Returns a new [`Bitmap`] that is a slice of this bitmap starting at `offset`.
    /// Doing so allows the same memory region to be shared between bitmaps.
    /// # Panics
    /// Panics iff `offset` is larger than `bit_len`.
    pub fn slice(&self, offset: usize) -> Self {
        assert!(
            offset <= self.bit_len(),
            "the offset of the new Bitmap cannot exceed the existing bit length"
        );
        Self {
            bits: self.bits.clone(),
            offset: self.offset + offset,
            length: self.length - offset,
        }
    }

    /// Returns a new [`Bitmap`] that is a slice of this buffer starting at `offset`,
    /// with `length` bits.
    /// Doing so allows the same memory region to be shared between bitmaps.
    /// # Panics
    /// Panics iff `(offset + length)` is larger than the existing bit_len.
    pub fn slice_with_length(&self, offset: usize, length: usize) -> Self {
        assert!(
            offset + length <= self.bit_len(),
            "the offset of the new Bitmap cannot exceed the existing bit length"
        );
        Self {
            bits: self.bits.clone(),
            offset: self.offset + offset,
            length,
        }
    }

    /// Returns the number of 1-bits in this bitmap, starting from `offset` with `len` bits
    /// inspected. Note that both `offset` and `len` are measured in bits.
    /// # Panics
    /// Panics iff `(offset + len)` is larger than the existing bit_len.
    pub fn count_set_bits_offset(&self, offset: usize, len: usize) -> usize {
        assert!(
            offset + len <= self.bit_len(),
            "the offset plus len cannot exceed the existing bit length"
        );
        UnalignedBitChunk::new(self.bits.as_slice(), self.offset + offset, len)
            .count_ones()
    }
}

impl<'a, 'b> BitAnd<&'b Bitmap> for &'a Bitmap {
    type Output = Result<Bitmap, ArrowError>;

    fn bitand(self, rhs: &'b Bitmap) -> Result<Bitmap, ArrowError> {
        if self.bits.len() != rhs.bits.len() {
            return Err(ArrowError::ComputeError(
                "Bitmaps must be the same size to apply Bitwise AND.".to_string(),
            ));
        }
        Ok(Bitmap::new_from_buffer(
            buffer_bin_and(
                &self.bits,
                self.offset,
                &rhs.bits,
                rhs.offset,
                self.bit_len(),
            ),
            0,
            self.bit_len(),
        ))
    }
}

impl<'a, 'b> BitOr<&'b Bitmap> for &'a Bitmap {
    type Output = Result<Bitmap, ArrowError>;

    fn bitor(self, rhs: &'b Bitmap) -> Result<Bitmap, ArrowError> {
        if self.bit_len() != rhs.bit_len() {
            return Err(ArrowError::ComputeError(
                "Bitmaps must be the same size to apply Bitwise OR.".to_string(),
            ));
        }
        Ok(Bitmap::new_from_buffer(
            buffer_bin_or(
                &self.bits,
                self.offset,
                &rhs.bits,
                rhs.offset,
                self.bit_len(),
            ),
            0,
            self.bit_len(),
        ))
    }
}

impl PartialEq for Bitmap {
    fn eq(&self, other: &Self) -> bool {
        // buffer equality considers capacity, but here we want to only compare
        // actual data contents
        let self_len = self.bit_len();
        let other_len = other.bit_len();
        if self_len != other_len {
            return false;
        }
        self.bits.as_slice()[self.offset..self_len]
            == other.bits.as_slice()[other.offset..self_len]
    }
}

impl AsRef<[u8]> for Bitmap {
    fn as_ref(&self) -> &[u8] {
        self.bits.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bitmap_length() {
        assert_eq!(512, Bitmap::new(63 * 8).bit_len());
        assert_eq!(512, Bitmap::new(64 * 8).bit_len());
        assert_eq!(1024, Bitmap::new(65 * 8).bit_len());
    }

    #[test]
    fn test_bitwise_and() {
        let bitmap1 = Bitmap::new_from_buffer(Buffer::from([0b01101010]), 0, 9);
        let bitmap2 = Bitmap::new_from_buffer(Buffer::from([0b01001110]), 0, 9);
        assert_eq!(
            Bitmap::from(Buffer::new_from_buffer([0b01001010], 0, 9)),
            (&bitmap1 & &bitmap2).unwrap()
        );
    }

    #[test]
    fn test_bitwise_or() {
        let bitmap1 = Bitmap::new_from_buffer(Buffer::from([0b01101010]), 0, 9);
        let bitmap2 = Bitmap::new_from_buffer(Buffer::from([0b01001110]), 0, 9);
        assert_eq!(
            Bitmap::new_from_buffer(Buffer::new_from_buffer([0b01101110]), 0, 9),
            (&bitmap1 | &bitmap2).unwrap()
        );
    }

    #[test]
    fn test_bitmap_is_set() {
        let bitmap = Bitmap::new_from_buffer(Buffer::from([0b01001010]), 0, 9);
        assert!(!bitmap.is_set(0));
        assert!(bitmap.is_set(1));
        assert!(!bitmap.is_set(2));
        assert!(bitmap.is_set(3));
        assert!(!bitmap.is_set(4));
        assert!(!bitmap.is_set(5));
        assert!(bitmap.is_set(6));
        assert!(!bitmap.is_set(7));
    }
}
