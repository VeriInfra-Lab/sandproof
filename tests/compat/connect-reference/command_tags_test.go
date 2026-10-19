// Fixed envd f455fd58fa3f289e78ce1de690c35968d8d29322 observations.
// service.go blob 889e484fd46f3da727c83d5d64370d27e0a05e06;
// connect.go blob b8373490414b9833932c29a5643d1472fab2580c.
// Apache-2.0 license: process/E2B-LICENSE.
//
// The three method bodies below are copied unchanged except receiver/record
// type names. Handler/retainedExit keep only fields those methods read.
// scheduledTagMap explicitly supplies possible sync.Map.Range visit orders;
// Load/Store/CompareAndDelete still use the real sync.Map. No random ordering,
// process execution, retention timer, HTTP, or Rust policy is tested here.
package main

import (
	"fmt"
	"reflect"
	"runtime"
	"strconv"
	"sync"
	"testing"

	"connectrpc.com/connect"
	rpc "sandproof.local/connect-reference/process"
)

type tagHandler struct{ Tag *string }
type tagExit struct {
	pid uint32
	tag *string
}
type tagReference struct {
	processes  *scheduledTagMap[*tagHandler]
	terminated *scheduledTagMap[*tagExit]
}

type scheduledTagMap[V comparable] struct {
	values         sync.Map
	order          []uint32
	visited        []uint32
	beforeCallback func(uint32, V)
}

func (m *scheduledTagMap[V]) Load(key uint32) (value V, ok bool) {
	raw, ok := m.values.Load(key)
	if !ok {
		return value, false
	}
	return raw.(V), true
}
func (m *scheduledTagMap[V]) Store(key uint32, value V) { m.values.Store(key, value) }
func (m *scheduledTagMap[V]) CompareAndDelete(key uint32, old V) bool {
	return m.values.CompareAndDelete(key, old)
}
func (m *scheduledTagMap[V]) Range(f func(uint32, V) bool) {
	for _, key := range m.order {
		value, ok := m.Load(key)
		if !ok {
			continue
		}
		m.visited = append(m.visited, key)
		if m.beforeCallback != nil {
			m.beforeCallback(key, value)
		}
		if !f(key, value) {
			return
		}
	}
}

func (s *tagReference) getProcess(selector *rpc.ProcessSelector) (*tagHandler, error) {
	var proc *tagHandler

	switch selector.GetSelector().(type) {
	case *rpc.ProcessSelector_Pid:
		p, ok := s.processes.Load(selector.GetPid())
		if !ok {
			return nil, connect.NewError(connect.CodeNotFound, fmt.Errorf("process with pid %d not found", selector.GetPid()))
		}

		proc = p
	case *rpc.ProcessSelector_Tag:
		tag := selector.GetTag()

		s.processes.Range(func(_ uint32, value *tagHandler) bool {
			if value.Tag == nil {
				return true
			}

			if *value.Tag == tag {
				proc = value

				return true
			}

			return false
		})

		if proc == nil {
			return nil, connect.NewError(connect.CodeNotFound, fmt.Errorf("process with tag %s not found", tag))
		}

	default:
		return nil, connect.NewError(connect.CodeUnimplemented, fmt.Errorf("invalid input type %T", selector))
	}

	return proc, nil
}

func (s *tagReference) lookupTerminated(selector *rpc.ProcessSelector) (*tagExit, bool) {
	switch selector.GetSelector().(type) {
	case *rpc.ProcessSelector_Pid:
		return s.terminated.Load(selector.GetPid())
	case *rpc.ProcessSelector_Tag:
		tag := selector.GetTag()

		var found *tagExit
		s.terminated.Range(func(_ uint32, v *tagExit) bool {
			if v.tag != nil && *v.tag == tag {
				found = v

				return false
			}

			return true
		})

		return found, found != nil
	default:
		return nil, false
	}
}

func (s *tagReference) clearTerminatedForTag(tag string) {
	s.terminated.Range(func(pid uint32, r *tagExit) bool {
		if r.tag != nil && *r.tag == tag {
			s.terminated.CompareAndDelete(pid, r)
		}

		return true
	})
}

func tagFixture(t *testing.T, order ...uint32) *tagReference {
	t.Helper()
	if runtime.Version() != "go1.26.8" || strconv.IntSize != 64 {
		t.Fatal("Go 1.26.8 / 64-bit int required")
	}
	tags := map[uint32]*string{1: nil, 2: tagText("wanted"), 3: tagText("other"), 4: tagText("wanted"), 5: tagText("")}
	s := &tagReference{
		processes:  &scheduledTagMap[*tagHandler]{order: order},
		terminated: &scheduledTagMap[*tagExit]{order: order},
	}
	for _, pid := range order {
		tag, ok := tags[pid]
		if !ok {
			t.Fatal("unknown fixture pid")
		}
		if _, duplicate := s.processes.Load(pid); duplicate {
			t.Fatal("duplicate fixture pid")
		}
		s.processes.Store(pid, &tagHandler{Tag: tag})
		s.terminated.Store(pid, &tagExit{pid: pid, tag: tag})
	}
	return s
}
func tagText(value string) *string { return &value }
func tagSelector(value string) *rpc.ProcessSelector {
	return &rpc.ProcessSelector{Selector: &rpc.ProcessSelector_Tag{Tag: value}}
}

func TestFixedGoLiveTagSelection(t *testing.T) {
	cases := []struct {
		name    string
		order   []uint32
		tag     string
		want    uint32
		visited []uint32
	}{
		{"empty-map", nil, "wanted", 0, nil},
		{"single-match", []uint32{2}, "wanted", 2, []uint32{2}},
		{"untagged-is-skipped", []uint32{1, 2}, "wanted", 2, []uint32{1, 2}},
		{"other-before-unique-match", []uint32{3, 2}, "wanted", 0, []uint32{3}},
		{"unique-match-before-other", []uint32{2, 3}, "wanted", 2, []uint32{2, 3}},
		{"duplicates-last-match", []uint32{2, 4}, "wanted", 4, []uint32{2, 4}},
		{"duplicates-reversed", []uint32{4, 2}, "wanted", 2, []uint32{4, 2}},
		{"other-between-duplicates", []uint32{2, 3, 4}, "wanted", 2, []uint32{2, 3}},
		{"untagged-between-duplicates", []uint32{2, 1, 4}, "wanted", 4, []uint32{2, 1, 4}},
		{"explicit-empty-tag", []uint32{1, 5}, "", 5, []uint32{1, 5}},
		{"empty-tag-still-stops-search", []uint32{5, 2}, "wanted", 0, []uint32{5}},
		{"missing-unicode-tag", []uint32{1, 3}, "不存在\n'标签", 0, []uint32{1, 3}},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			s := tagFixture(t, tc.order...)
			got, err := s.getProcess(tagSelector(tc.tag))
			if tc.want == 0 {
				if got != nil || connect.CodeOf(err) != connect.CodeNotFound ||
					err.Error() != "not_found: process with tag "+tc.tag+" not found" {
					t.Fatalf("got (%v, %v)", got, err)
				}
			} else {
				want, _ := s.processes.Load(tc.want)
				if got != want || err != nil {
					t.Fatalf("got (%p, %v), want %p", got, err, want)
				}
			}
			if !reflect.DeepEqual(s.processes.visited, tc.visited) {
				t.Fatalf("visited %v, want %v", s.processes.visited, tc.visited)
			}
		})
	}
}

func TestFixedGoRetainedTagSelection(t *testing.T) {
	cases := []struct {
		name    string
		order   []uint32
		tag     string
		want    uint32
		visited []uint32
	}{
		{"other-before-unique-match", []uint32{3, 2}, "wanted", 2, []uint32{3, 2}},
		{"duplicates-first-match", []uint32{2, 4}, "wanted", 2, []uint32{2}},
		{"duplicates-reversed", []uint32{4, 2}, "wanted", 4, []uint32{4}},
		{"explicit-empty-tag", []uint32{1, 3, 5}, "", 5, []uint32{1, 3, 5}},
		{"missing-tag", []uint32{1, 2, 3, 4, 5}, "absent", 0, []uint32{1, 2, 3, 4, 5}},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			s := tagFixture(t, tc.order...)
			got, ok := s.lookupTerminated(tagSelector(tc.tag))
			want, _ := s.terminated.Load(tc.want)
			if got != want || ok != (tc.want != 0) {
				t.Fatalf("got (%v, %v), want %v", got, ok, want)
			}
			if !reflect.DeepEqual(s.terminated.visited, tc.visited) {
				t.Fatalf("visited %v, want %v", s.terminated.visited, tc.visited)
			}
		})
	}
}

func TestFixedGoTagClearKeepsReplacement(t *testing.T) {
	s := tagFixture(t, 2, 3, 1, 4, 5)
	replacement := &tagExit{pid: 4, tag: tagText("wanted")}
	s.terminated.beforeCallback = func(pid uint32, old *tagExit) {
		if pid == 4 {
			if old == replacement {
				t.Fatal("replacement must occur after Range reads the old pointer")
			}
			s.terminated.Store(pid, replacement)
		}
	}
	s.clearTerminatedForTag("wanted")
	if _, ok := s.terminated.Load(2); ok {
		t.Fatal("matching old entry survived")
	}
	if got, _ := s.terminated.Load(4); got != replacement {
		t.Fatal("replacement was deleted")
	}
	for _, pid := range []uint32{1, 3, 5} {
		if _, ok := s.terminated.Load(pid); !ok {
			t.Fatalf("unrelated entry %d deleted", pid)
		}
	}
	s.terminated.beforeCallback = nil
	s.clearTerminatedForTag("wanted")
	if _, ok := s.terminated.Load(4); ok {
		t.Fatal("new matching entry survived a later clear")
	}
	s.clearTerminatedForTag("")
	if _, ok := s.terminated.Load(5); ok {
		t.Fatal("explicit empty tag survived")
	}
	if _, ok := s.terminated.Load(1); !ok {
		t.Fatal("nil tag conflated with empty")
	}
}

func TestFixedGoPIDAndMissingSelector(t *testing.T) {
	s := tagFixture(t, 3, 2)
	selector := &rpc.ProcessSelector{Selector: &rpc.ProcessSelector_Pid{Pid: 2}}
	got, err := s.getProcess(selector)
	want, _ := s.processes.Load(2)
	if got != want || err != nil {
		t.Fatal("PID lookup changed")
	}
	retained, ok := s.lookupTerminated(selector)
	if !ok || retained.pid != 2 {
		t.Fatal("retained PID lookup changed")
	}
	for _, selector := range []*rpc.ProcessSelector{nil, {}} {
		got, err := s.getProcess(selector)
		if got != nil || connect.CodeOf(err) != connect.CodeUnimplemented ||
			err.Error() != "unimplemented: invalid input type *process.ProcessSelector" {
			t.Fatalf("unexpected missing selector: %v, %v", got, err)
		}
		if got, ok := s.lookupTerminated(selector); got != nil || ok {
			t.Fatal("missing selector resolved")
		}
	}
	selector = &rpc.ProcessSelector{Selector: &rpc.ProcessSelector_Pid{Pid: 99}}
	if got, err := s.getProcess(selector); got != nil || err.Error() != "not_found: process with pid 99 not found" {
		t.Fatalf("unexpected missing pid: %v, %v", got, err)
	}
	if got, ok := s.lookupTerminated(selector); got != nil || ok {
		t.Fatal("missing PID retained")
	}
	if len(s.processes.visited) != 0 || len(s.terminated.visited) != 0 {
		t.Fatal("PID/missing selector traversed tags")
	}
}
