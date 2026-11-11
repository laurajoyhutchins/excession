package main

import (
	"strings"
	"testing"
)

const eqBase = `package sets
func (s1 Set[T]) Equal(s2 Set[T]) bool {
 return len(s1) == len(s2) && s1.IsSuperset(s2)
}
`
const eqLeft = `package sets
func (s1 Set[T]) Equal(s2 Set[T]) bool {
 return s1.Len() == s2.Len() && s1.IsSuperset(s2)
}
`
const eqRight = `package sets
func (s1 Set[T]) Equal(s2 Set[T]) bool {
 return len(s1) == len(s2) && s2.IsSuperset(s1)
}
`

func TestGoGenericsMethodComposition(t *testing.T) {
	out, err := infer(request{Base: eqBase, Left: eqLeft, Right: eqRight, Qualname: "Set.Equal", Index: 0})
	if err != nil {
		t.Fatal(err)
	}
	expected := strings.Replace(eqLeft, "s1.IsSuperset(s2)", "s2.IsSuperset(s1)", 1)
	if out["content"] != expected {
		t.Fatalf("unexpected composition: %q", out["content"])
	}
	if out["source_confluence"] != true {
		t.Fatal("missing exact confluence witness")
	}
}
func TestGoStandaloneFunctionComposition(t *testing.T) {
	base := "package calc\nfunc Count(a,b []int) bool { return len(a)==len(b) && check(a,b) }\n"
	left := "package calc\nfunc Count(a,b []int) bool { return size(a)==len(b) && check(a,b) }\n"
	right := "package calc\nfunc Count(a,b []int) bool { return len(a)==len(b) && check(b,a) }\n"
	out, err := infer(request{Base: base, Left: left, Right: right, Qualname: "Count", Index: 0})
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(out["content"].(string), "size(a)==len(b) && check(b,a)") {
		t.Fatal("missing effect")
	}
}
func TestGoRefuseWrongStatementTarget(t *testing.T) {
	_, err := infer(request{Base: eqBase, Left: eqLeft, Right: eqRight, Qualname: "Set.Equal", Index: 1})
	if err == nil {
		t.Fatal("accepted nonexistent statement")
	}
}
func TestGoRefuseHiddenEdit(t *testing.T) {
	_, err := infer(request{Base: eqBase, Left: eqLeft, Right: eqRight + "// hidden source edit\n", Qualname: "Set.Equal", Index: 0})
	if err == nil || !strings.Contains(err.Error(), "outside") {
		t.Fatalf("should refuse hidden edit: %v", err)
	}
}
func TestGoRefuseSameSpliceWithoutContext(t *testing.T) {
	_, err := infer(request{Base: eqBase, Left: eqRight, Right: eqRight, Qualname: "Set.Equal", Index: 0})
	if err == nil {
		t.Fatal("accepted duplicate independent edits")
	}
}
func TestGoRefuseInvalidParserInput(t *testing.T) {
	_, err := infer(request{Base: eqBase, Left: eqLeft, Right: "package sets\nfunc (\n", Qualname: "Set.Equal", Index: 0})
	if err == nil {
		t.Fatal("accepted invalid Go source")
	}
}
func TestGoPreserveUntouchedComments(t *testing.T) {
	withHeader := func(s string) string { return "// Provenance comment Δ 🧭\n" + s }
	out, err := infer(request{Base: withHeader(eqBase), Left: withHeader(eqLeft), Right: withHeader(eqRight), Qualname: "Set.Equal", Index: 0})
	if err != nil {
		t.Fatal(err)
	}
	if !strings.HasPrefix(out["content"].(string), "// Provenance comment Δ 🧭\n") {
		t.Fatal("lost metadata bytes")
	}
}
func TestGoRefuseAmbiguousAnchors(t *testing.T) {
	// The same call appears twice in the context branch; unanchored movement is unsafe.
	b := "package a\nfunc F(a,b int) int { return a + b }\n"
	l := "package a\nfunc F(a,b int) int { return (a + b) + (a + b) }\n"
	r := "package a\nfunc F(a,b int) int { return a - b }\n"
	_, err := infer(request{Base: b, Left: l, Right: r, Qualname: "F", Index: 0})
	if err == nil {
		t.Fatal("accepted ambiguous/syntax-divergent edit")
	}
}
