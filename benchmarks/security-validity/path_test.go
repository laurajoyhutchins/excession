package validity

import (
	"os"
	"path/filepath"
	"testing"
)

func sample(t *testing.T) (string, []byte) {
	t.Helper()
	tmp := t.TempDir()
	root := filepath.Join(tmp, "tenant")
	if err := os.Mkdir(root, 0700); err != nil {
		t.Fatal(err)
	}
	secret := []byte("not-authorized")
	if err := os.WriteFile(filepath.Join(tmp, "secret.txt"), secret, 0600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(root, "public.txt"), []byte("authorized"), 0600); err != nil {
		t.Fatal(err)
	}
	return root, secret
}

// The positive oracle asserts an actual outside-root read, not a matched pattern.
func TestTraversalIsObservable(t *testing.T) {
	root, secret := sample(t)
	got, err := ReadUntrusted(root, "../secret.txt")
	if err != nil || string(got) != string(secret) {
		t.Fatalf("outside-root read not reproduced: %q, %v", got, err)
	}
}

func TestGuardRefusesTraversal(t *testing.T) {
	root, secret := sample(t)
	got, err := ReadGuarded(root, "../secret.txt")
	if err == nil || string(got) == string(secret) {
		t.Fatalf("guard failed: %q, %v", got, err)
	}
}

func TestGuardPreservesBenignAccess(t *testing.T) {
	root, _ := sample(t)
	got, err := ReadGuarded(root, "public.txt")
	if err != nil || string(got) != "authorized" {
		t.Fatalf("benign path failed: %q, %v", got, err)
	}
}

func TestConstantReadIsBenign(t *testing.T) {
	root, _ := sample(t)
	got, err := ReadConstant(root)
	if err != nil || string(got) != "authorized" {
		t.Fatalf("constant read failed: %q, %v", got, err)
	}
}
