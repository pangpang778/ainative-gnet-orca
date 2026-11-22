// Copyright (c) 2026 ainative-gnet-orca contributors.
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

// Go differential probe for U2: elastic buffer + linkedlist buffer.
// Emits one JSON object per op; the Rust twin (crates/gnet-core/examples/
// elastic_probe.rs) must produce byte-identical output for the same
// xorshift64* seed.
package main

import (
	"bytes"
	"fmt"
	"os"

	"github.com/panjf2000/gnet/v2/pkg/buffer/elastic"
	"github.com/panjf2000/gnet/v2/pkg/buffer/linkedlist"
)

// xorshift64* — must match the Rust probe's generator exactly.
type xs struct {
	s uint64
}

func (g *xs) next() uint64 {
	x := g.s
	x ^= x >> 12
	x ^= x << 25
	x ^= x >> 27
	g.s = x
	return x * 0x2545F4914F6CDD1D
}

func (g *xs) bytes(n int) []byte {
	out := make([]byte, 0, n)
	for len(out) < n {
		v := g.next()
		var b [8]byte
		for i := 0; i < 8; i++ {
			b[i] = byte(v >> (8 * uint(i)))
		}
		for i := 0; i < 8 && len(out) < n; i++ {
			out = append(out, b[i])
		}
	}
	return out
}

func hex(b []byte) string {
	s := ""
	for _, c := range b {
		s += fmt.Sprintf("%02x", c)
	}
	return s
}

func emit(s string) {
	fmt.Println(s)
}

func main() {
	g := &xs{s: 0x9E3779B97F4A7C15 | 1}
	var err error

	// ===== Scenario A: linkedlist basic =====
	var llb linkedlist.Buffer
	cum := 0
	var expected bytes.Buffer
	for i := 0; i < 20; i++ {
		n := int(g.next()%1024) + 128
		cum += n
		data := g.bytes(n)
		llb.PushBack(data)
		expected.Write(data)
	}
	emit(fmt.Sprintf(`{"buffered":%d,"len":%d,"op":"ll_pushback"}`, llb.Buffered(), llb.Len()))

	bs, err := llb.Peek(cum / 4)
	if err != nil {
		emit(fmt.Sprintf(`{"err":"%v","op":"ll_peek"}`, err))
	} else {
		var p bytes.Buffer
		for _, b := range bs {
			p.Write(b)
		}
		emit(fmt.Sprintf(`{"n":%d,"op":"ll_peek","total":"%s"}`, cum/4, hex(p.Bytes())))
	}

	tmpA := g.bytes(cum / 16)
	tmpB := g.bytes(cum / 16)
	bs, err = llb.PeekWithBytes(cum/4, tmpA, tmpB)
	if err != nil {
		emit(fmt.Sprintf(`{"err":"%v","op":"ll_peekwith"}`, err))
	} else {
		var p bytes.Buffer
		for _, b := range bs {
			p.Write(b)
		}
		emit(fmt.Sprintf(`{"n":%d,"op":"ll_peekwith","total":"%s"}`, cum/4, hex(p.Bytes())))
	}

	pn, _ := llb.Discard(cum / 4)
	emit(fmt.Sprintf(`{"buffered":%d,"discarded":%d,"op":"ll_discard"}`, llb.Buffered(), pn))

	out := make([]byte, cum-pn)
	n, err := llb.Read(out)
	if err != nil {
		emit(fmt.Sprintf(`{"err":"%v","n":%d,"op":"ll_read"}`, err, n))
	} else {
		emit(fmt.Sprintf(`{"data":"%s","n":%d,"op":"ll_read"}`, hex(out), n))
	}
	emit(fmt.Sprintf(`{"is_empty":%v,"op":"ll_state"}`, llb.IsEmpty()))

	// ===== Scenario B: linkedlist ReadFrom + WriteTo =====
	llb.Reset()
	data := g.bytes(2048)
	r := bytes.NewReader(data)
	rn, err := llb.ReadFrom(r)
	emit(fmt.Sprintf(`{"err":%v,"n":%d,"op":"ll_readfrom"}`, err != nil, rn))
	var wbuf bytes.Buffer
	wn, err := llb.WriteTo(&wbuf)
	emit(fmt.Sprintf(`{"data":"%s","err":%v,"n":%d,"op":"ll_writeto"}`, hex(wbuf.Bytes()), err != nil, wn))

	// WriteTo with a short writer (accepts only 100 bytes).
	llb.Reset()
	for i := 0; i < 3; i++ {
		llb.PushBack(g.bytes(200))
	}
	sw := &shortWriter{n: 100}
	wn, err = llb.WriteTo(sw)
	emit(fmt.Sprintf(`{"err":%v,"n":%d,"op":"ll_writeto_short"}`, err != nil, wn))
	emit(fmt.Sprintf(`{"buffered":%d,"len":%d,"op":"ll_state_after_short"}`, llb.Buffered(), llb.Len()))

	// ===== Scenario C: elastic basic (ring only, under maxStatic) =====
	mb, err := elastic.New(4096)
	if err != nil {
		emit(fmt.Sprintf(`{"err":"%v","op":"new_err"}`, err))
		os.Exit(1)
	}
	data = g.bytes(2048)
	n, err = mb.Write(data)
	emit(fmt.Sprintf(`{"buffered":%d,"err":%v,"n":%d,"op":"e_write"}`,
		mb.Buffered(), err != nil, n))

	// Ring peek emulation: list is empty, so mb.Peek(buffered) returns the ring's
	// [head, tail] chunks exactly as elastic.Peek's ring branch does.
	chunks, _ := mb.Peek(-1)
	var head, tail []byte
	if len(chunks) > 0 {
		head = chunks[0]
	}
	if len(chunks) > 1 {
		tail = chunks[1]
	}
	emit(fmt.Sprintf(`{"head":"%s","head_len":%d,"op":"e_ringpeek","tail":"%s","tail_len":%d}`,
		hex(head), len(head), hex(tail), len(tail)))

	n, err = mb.Read(make([]byte, 1024))
	emit(fmt.Sprintf(`{"buffered":%d,"err":%v,"n":%d,"op":"e_read"}`, mb.Buffered(), err, n))

	// ===== Scenario D: elastic hybrid (overflow to list) =====
	mb.Reset(2048)
	big := g.bytes(6144)
	n, _ = mb.Write(big)
	emit(fmt.Sprintf(`{"buffered":%d,"n":%d,"op":"e_write_hybrid"}`, mb.Buffered(), n))

	bs, err = mb.Peek(-1)
	if err != nil {
		emit(fmt.Sprintf(`{"err":"%v","op":"e_peek_all"}`, err))
	} else {
		var p bytes.Buffer
		for _, b := range bs {
			p.Write(b)
		}
		emit(fmt.Sprintf(`{"nchunks":%d,"op":"e_peek_all","total":"%s"}`, len(bs), hex(p.Bytes())))
	}

	bs, err = mb.Peek(3000)
	if err != nil {
		emit(fmt.Sprintf(`{"err":"%v","op":"e_peek_3000"}`, err))
	} else {
		var p bytes.Buffer
		for _, b := range bs {
			p.Write(b)
		}
		emit(fmt.Sprintf(`{"nchunks":%d,"op":"e_peek_3000","total":"%s"}`, len(bs), hex(p.Bytes())))
	}

	// Writev over the threshold.
	mb.Reset(2048)
	var parts [][]byte
	cum2 := 0
	for i := 0; i < 6; i++ {
		p := g.bytes(1000)
		cum2 += 1000
		parts = append(parts, p)
	}
	nv, _ := mb.Writev(parts)
	emit(fmt.Sprintf(`{"buffered":%d,"n":%d,"op":"e_writev"}`, mb.Buffered(), nv))

	bs, err = mb.Peek(-1)
	var all bytes.Buffer
	for _, b := range bs {
		all.Write(b)
	}
	emit(fmt.Sprintf(`{"err":%v,"nchunks":%d,"op":"e_peek_after_writev","total":"%s"}`, err != nil, len(bs), hex(all.Bytes())))

	// WriteTo drains everything.
	var wbuf2 bytes.Buffer
	wn, err = mb.WriteTo(&wbuf2)
	emit(fmt.Sprintf(`{"data_match":%v,"err":%v,"n":%d,"op":"e_writeto"}`,
		bytes.Equal(wbuf2.Bytes(), all.Bytes()), err != nil, wn))
	emit(fmt.Sprintf(`{"is_empty":%v,"op":"e_state"}`, mb.IsEmpty()))

	// Discard across ring/list boundary.
	mb.Reset(2048)
	d1 := g.bytes(3000)
	_, _ = mb.Write(d1)
	dd, _ := mb.Discard(2500)
	emit(fmt.Sprintf(`{"buffered":%d,"discarded":%d,"op":"e_discard"}`, mb.Buffered(), dd))
	rest, _ := mb.Peek(-1)
	var rb bytes.Buffer
	for _, b := range rest {
		rb.Write(b)
	}
	emit(fmt.Sprintf(`{"rest":"%s","op":"e_rest"}`, hex(rb.Bytes())))

	// ReadFrom into elastic.
	mb.Reset(2048)
	rf := g.bytes(4000)
	r2 := bytes.NewReader(rf)
	rn, err = mb.ReadFrom(r2)
	emit(fmt.Sprintf(`{"buffered":%d,"err":%v,"n":%d,"op":"e_readfrom"}`,
		mb.Buffered(), err != nil, rn))
}

type shortWriter struct {
	n int
}

func (s *shortWriter) Write(p []byte) (int, error) {
	if len(p) > s.n {
		w := s.n
		s.n = 0
		return w, nil
	}
	s.n -= len(p)
	return len(p), nil
}
