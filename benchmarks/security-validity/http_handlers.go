package validity

import "net/http"

// HTTPUntrustedRead deliberately models a user-controlled path reaching a file read.
// This is a scanner benchmark, not production server code.
func HTTPUntrustedRead(root string) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		member := r.URL.Query().Get("name")
		contents, err := ReadUntrusted(root, member)
		if err != nil {
			http.Error(w, "read failed", http.StatusNotFound)
			return
		}
		_, _ = w.Write(contents)
	})
}

// HTTPGuardedRead routes the same external input through the guarded control.
func HTTPGuardedRead(root string) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		member := r.URL.Query().Get("name")
		contents, err := ReadGuarded(root, member)
		if err != nil {
			http.Error(w, "read rejected", http.StatusBadRequest)
			return
		}
		_, _ = w.Write(contents)
	})
}
