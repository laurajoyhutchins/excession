package validity

import (
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"
)

func httpSample(t *testing.T) (string, []byte) {
	t.Helper()
	temp := t.TempDir()
	root := filepath.Join(temp, "tenant")
	if err := os.Mkdir(root, 0700); err != nil {
		t.Fatal(err)
	}
	secret := []byte("outside-root-secret")
	if err := os.WriteFile(filepath.Join(temp, "secret.txt"), secret, 0600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(root, "public.txt"), []byte("public"), 0600); err != nil {
		t.Fatal(err)
	}
	return root, secret
}

func TestHTTPTraversalIsObservable(t *testing.T) {
	root, secret := httpSample(t)
	got := httptest.NewRecorder()
	HTTPUntrustedRead(root).ServeHTTP(got, httptest.NewRequest(http.MethodGet, "/?name=..%2Fsecret.txt", nil))
	if got.Code != http.StatusOK || got.Body.String() != string(secret) {
		t.Fatalf("outside-root secret not exposed by HTTP: %d %q", got.Code, got.Body.String())
	}
}

func TestHTTPGuardRefusesTraversal(t *testing.T) {
	root, secret := httpSample(t)
	got := httptest.NewRecorder()
	HTTPGuardedRead(root).ServeHTTP(got, httptest.NewRequest(http.MethodGet, "/?name=..%2Fsecret.txt", nil))
	if got.Code != http.StatusBadRequest || got.Body.String() == string(secret) {
		t.Fatalf("guard leaked secret: %d %q", got.Code, got.Body.String())
	}
}

func TestHTTPGuardPreservesBenignAccess(t *testing.T) {
	root, _ := httpSample(t)
	got := httptest.NewRecorder()
	HTTPGuardedRead(root).ServeHTTP(got, httptest.NewRequest(http.MethodGet, "/?name=public.txt", nil))
	if got.Code != http.StatusOK || got.Body.String() != "public" {
		t.Fatalf("guard denied benign access: %d %q", got.Code, got.Body.String())
	}
}
