// Package validity is a deliberately limited local path traversal benchmark.
// Do not reuse these functions in production code.
package validity

import (
	"errors"
	"os"
	"path/filepath"
	"strings"
)

// ReadUntrusted intentionally permits a parent-component escape.
func ReadUntrusted(root, member string) ([]byte, error) {
	return os.ReadFile(filepath.Join(root, member))
}

// ReadGuarded prevents parent-component escape in this local benchmark.
// It is not a symlink-safe filesystem containment implementation.
func ReadGuarded(root, member string) ([]byte, error) {
	joined := filepath.Join(root, member)
	rel, err := filepath.Rel(root, joined)
	if err != nil || rel == ".." || strings.HasPrefix(rel, ".."+string(filepath.Separator)) {
		return nil, errors.New("outside root")
	}
	return os.ReadFile(joined)
}

// ReadConstant has the same syntactic file-read shape, but no attacker input.
func ReadConstant(root string) ([]byte, error) {
	return os.ReadFile(filepath.Join(root, "public.txt"))
}
