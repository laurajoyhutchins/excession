// Go-native, exact-source subexpression transport for RelGit research.
// Syntax analysis uses go/parser, not ad hoc Go text matching.
// This proves one bounded textual confluence relation, not program equivalence.
package main

import (
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"go/ast"
	"go/parser"
	"go/token"
	"io"
	"os"
	"strings"
)

type request struct {
	Base     string `json:"base"`
	Left     string `json:"left"`
	Right    string `json:"right"`
	Qualname string `json:"qualname"`
	Index    int    `json:"index"`
}
type location struct {
	text       string
	fset       *token.FileSet
	expr       ast.Expr
	stmt       ast.Stmt
	start, end int
}
type node struct {
	start, end, depth int
	kind, source      string
}

func hash(s string) string                        { x := sha256.Sum256([]byte(s)); return hex.EncodeToString(x[:]) }
func offset(fs *token.FileSet, pos token.Pos) int { return fs.PositionFor(pos, false).Offset }
func receiverName(e ast.Expr) string {
	switch t := e.(type) {
	case *ast.Ident:
		return t.Name
	case *ast.StarExpr:
		return receiverName(t.X)
	case *ast.IndexExpr:
		return receiverName(t.X)
	case *ast.IndexListExpr:
		return receiverName(t.X)
	default:
		return ""
	}
}
func locate(text, target string, ix int) (location, error) {
	var out location
	if ix < 0 || ix > 500 {
		return out, errors.New("invalid statement index")
	}
	parts := strings.Split(target, ".")
	if len(parts) < 1 || len(parts) > 2 {
		return out, errors.New("Go target must be Function or Receiver.Method")
	}
	for _, p := range parts {
		if p == "" {
			return out, errors.New("empty Go target component")
		}
	}
	fset := token.NewFileSet()
	file, err := parser.ParseFile(fset, "relgit.go", text, parser.AllErrors)
	if err != nil {
		return out, fmt.Errorf("Go parse: %w", err)
	}
	var found *ast.FuncDecl
	for _, decl := range file.Decls {
		d, ok := decl.(*ast.FuncDecl)
		if !ok || d.Name.Name != parts[len(parts)-1] {
			continue
		}
		if len(parts) == 1 && d.Recv != nil {
			continue
		}
		if len(parts) == 2 && (d.Recv == nil || len(d.Recv.List) != 1 || receiverName(d.Recv.List[0].Type) != parts[0]) {
			continue
		}
		if found != nil {
			return out, errors.New("ambiguous duplicate Go declaration")
		}
		found = d
	}
	if found == nil || found.Body == nil || ix >= len(found.Body.List) {
		return out, errors.New("Go statement not found")
	}
	stmt := found.Body.List[ix]
	ret, ok := stmt.(*ast.ReturnStmt)
	if !ok || len(ret.Results) != 1 {
		return out, errors.New("only single-value Go return supported")
	}
	start, end := offset(fset, stmt.Pos()), offset(fset, stmt.End())
	if start < 0 || end > len(text) || start >= end {
		return out, errors.New("invalid Go statement span")
	}
	return location{text: text, fset: fset, expr: ret.Results[0], stmt: stmt, start: start, end: end}, nil
}
func nodes(l location) []node {
	out := []node{}
	// Inspect walks child nodes in lexical preorder; depth is tracked using stack of end positions.
	stack := []int{}
	ast.Inspect(l.expr, func(n ast.Node) bool {
		if n == nil {
			if len(stack) > 0 {
				stack = stack[:len(stack)-1]
			}
			return false
		}
		depth := len(stack)
		stack = append(stack, offset(l.fset, n.End()))
		if _, ok := n.(ast.Expr); ok {
			b, e := offset(l.fset, n.Pos()), offset(l.fset, n.End())
			if b >= 0 && b < e && e <= len(l.text) {
				out = append(out, node{start: b, end: e, depth: depth, kind: fmt.Sprintf("%T", n), source: l.text[b:e]})
			}
		}
		return true
	})
	return out
}
func sameOutside(base, changed location) bool {
	return base.text[:base.start] == changed.text[:changed.start] && base.text[base.end:] == changed.text[changed.end:]
}
func infer(r request) (map[string]any, error) {
	b, err := locate(r.Base, r.Qualname, r.Index)
	if err != nil {
		return nil, err
	}
	l, err := locate(r.Left, r.Qualname, r.Index)
	if err != nil {
		return nil, err
	}
	rr, err := locate(r.Right, r.Qualname, r.Index)
	if err != nil {
		return nil, err
	}
	if !sameOutside(b, l) || !sameOutside(b, rr) {
		return nil, errors.New("undeclared edit outside target Go statement")
	}
	if b.text[b.start:b.end] == l.text[l.start:l.end] || b.text[b.start:b.end] == rr.text[rr.start:rr.end] {
		return nil, errors.New("branch made no statement edit")
	}
	var candidates []struct {
		old, nodeNew node
		merged       string
	}
	for _, old := range nodes(b) {
		// Right branch must be exactly one replacement of this AST expression,
		// including the entire remainder of the tracked source file.
		prefix, suffix := b.text[:old.start], b.text[old.end:]
		if !strings.HasPrefix(rr.text, prefix) || !strings.HasSuffix(rr.text, suffix) {
			continue
		}
		changed := rr.text[len(prefix) : len(rr.text)-len(suffix)]
		if changed == "" || changed == old.source {
			continue
		}
		var rhs []node
		for _, n := range nodes(rr) {
			if n.source == changed && n.start == old.start {
				rhs = append(rhs, n)
			}
		}
		if len(rhs) != 1 {
			continue
		}
		var anchors []node
		for _, n := range nodes(l) {
			if n.kind == old.kind && n.source == old.source {
				anchors = append(anchors, n)
			}
		}
		if len(anchors) != 1 {
			continue
		}
		a := anchors[0]
		merged := l.text[:a.start] + changed + l.text[a.end:]
		result, err := locate(merged, r.Qualname, r.Index)
		if err != nil {
			continue
		}
		if !sameOutside(l, result) {
			continue
		}
		other := rr.text[:rr.start] + l.text[l.start:a.start] + changed + l.text[a.end:l.end] + rr.text[rr.end:]
		if merged != other {
			continue
		}
		candidates = append(candidates, struct {
			old, nodeNew node
			merged       string
		}{old, rhs[0], merged})
	}
	if len(candidates) == 0 {
		return nil, errors.New("no uniquely transportable Go expression")
	}
	best := candidates[0]
	for _, c := range candidates[1:] {
		if c.old.depth > best.old.depth {
			best = c
		}
	}
	count := 0
	for _, c := range candidates {
		if c.old.depth == best.old.depth {
			count++
		}
	}
	if count != 1 {
		return nil, errors.New("ambiguous Go effect anchors")
	}
	m, err := locate(best.merged, r.Qualname, r.Index)
	if err != nil {
		return nil, err
	}
	return map[string]any{
		"content":                 best.merged,
		"calculus":                "UNIQUE_SUBEXPRESSION_CONTEXT_V1",
		"statement_index":         r.Index,
		"anchor_before_sha256":    hash(best.old.source),
		"anchor_after_sha256":     hash(best.nodeNew.source),
		"base_statement_sha256":   hash(b.text[b.start:b.end]),
		"left_statement_sha256":   hash(l.text[l.start:l.end]),
		"right_statement_sha256":  hash(rr.text[rr.start:rr.end]),
		"merged_statement_sha256": hash(m.text[m.start:m.end]),
		"anchor_path":             []any{[]any{"go_ast_depth", best.old.depth}, []any{"byte_offset_in_stmt", best.old.start - b.start}},
		"source_confluence":       true,
	}, nil
}
func main() {
	buf, err := io.ReadAll(io.LimitReader(os.Stdin, 1024*1024+1))
	if err != nil || len(buf) > 1024*1024 {
		fmt.Fprintln(os.Stderr, "GO_INFER_REFUSAL: request too large")
		os.Exit(2)
	}
	var r request
	if err = json.Unmarshal(buf, &r); err != nil {
		fmt.Fprintln(os.Stderr, "GO_INFER_REFUSAL:", err)
		os.Exit(2)
	}
	if !bytes.Equal([]byte(r.Base), []byte(strings.ToValidUTF8(r.Base, ""))) {
		fmt.Fprintln(os.Stderr, "GO_INFER_REFUSAL: invalid UTF-8")
		os.Exit(2)
	}
	out, err := infer(r)
	if err != nil {
		fmt.Fprintln(os.Stderr, "GO_INFER_REFUSAL:", err)
		os.Exit(2)
	}
	if err = json.NewEncoder(os.Stdout).Encode(out); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(2)
	}
}
