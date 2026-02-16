// Copyright 2026 Datafuse Labs.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::arch::x86_64::*;

#[inline(always)]
pub(crate) fn skip_json_whitespace(buf: &[u8], idx: usize) -> usize {
    // SAFETY: The crate is compiled with x86-64-v3, so AVX2 is guaranteed.
    unsafe { skip_json_whitespace_avx2(buf, idx) }
}

#[target_feature(enable = "avx2")]
unsafe fn skip_json_whitespace_avx2(buf: &[u8], mut idx: usize) -> usize {
    let len = buf.len();
    if idx >= len {
        return idx;
    }
    if !matches!(buf[idx], b' ' | b'\n' | b'\r' | b'\t') {
        return idx;
    }

    if len - idx < 32 {
        while idx < len && matches!(buf[idx], b' ' | b'\n' | b'\r' | b'\t') {
            idx += 1;
        }
        return idx;
    }

    let ws_space = _mm256_set1_epi8(b' ' as i8);
    let ws_newline = _mm256_set1_epi8(b'\n' as i8);
    let ws_carriage = _mm256_set1_epi8(b'\r' as i8);
    let ws_tab = _mm256_set1_epi8(b'\t' as i8);

    while idx + 32 <= len {
        let ptr = unsafe { buf.as_ptr().add(idx) as *const __m256i };
        let chunk = unsafe { _mm256_loadu_si256(ptr) };

        let eq_space = _mm256_cmpeq_epi8(chunk, ws_space);
        let eq_newline = _mm256_cmpeq_epi8(chunk, ws_newline);
        let eq_carriage = _mm256_cmpeq_epi8(chunk, ws_carriage);
        let eq_tab = _mm256_cmpeq_epi8(chunk, ws_tab);

        let ws_mask = _mm256_or_si256(
            _mm256_or_si256(eq_space, eq_newline),
            _mm256_or_si256(eq_carriage, eq_tab),
        );
        let mask = _mm256_movemask_epi8(ws_mask) as u32;

        if mask == u32::MAX {
            idx += 32;
            continue;
        }

        idx += (!mask).trailing_zeros() as usize;
        return idx;
    }

    while idx < len && matches!(buf[idx], b' ' | b'\n' | b'\r' | b'\t') {
        idx += 1;
    }
    idx
}

#[inline(always)]
pub(crate) fn find_byte2(buf: &[u8], idx: usize, c1: u8, c2: u8) -> Option<usize> {
    // SAFETY: The crate is compiled with x86-64-v3, so AVX2 is guaranteed.
    unsafe { find_byte2_avx2(buf, idx, c1, c2) }
}

#[target_feature(enable = "avx2")]
unsafe fn find_byte2_avx2(buf: &[u8], mut idx: usize, c1: u8, c2: u8) -> Option<usize> {
    let len = buf.len();
    if idx >= len {
        return None;
    }
    if len - idx < 32 {
        while idx < len {
            let byte = unsafe { *buf.get_unchecked(idx) };
            if byte == c1 || byte == c2 {
                return Some(idx);
            }
            idx += 1;
        }
        return None;
    }

    let needle1 = _mm256_set1_epi8(c1 as i8);
    let needle2 = _mm256_set1_epi8(c2 as i8);

    while idx + 32 <= len {
        let ptr = unsafe { buf.as_ptr().add(idx) as *const __m256i };
        let chunk = unsafe { _mm256_loadu_si256(ptr) };
        let hit1 = _mm256_cmpeq_epi8(chunk, needle1);
        let hit2 = _mm256_cmpeq_epi8(chunk, needle2);
        let mask = _mm256_movemask_epi8(_mm256_or_si256(hit1, hit2)) as u32;
        if mask != 0 {
            return Some(idx + mask.trailing_zeros() as usize);
        }
        idx += 32;
    }

    while idx < len {
        let byte = unsafe { *buf.get_unchecked(idx) };
        if byte == c1 || byte == c2 {
            return Some(idx);
        }
        idx += 1;
    }
    None
}
