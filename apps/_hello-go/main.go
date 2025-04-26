package main

import (
	"fmt"
	"net/http"
	"os"

	spinhttp "github.com/fermyon/spin/sdk/go/v2/http"
)

func init() {
	// spinhttp.Handle(func(w http.ResponseWriter, r *http.Request) {
	// 	w.Header().Set("Content-Type", "text/plain")
	// 	fmt.Fprintln(w, "Hello Fermyon!")
	// })

	spinhttp.Handle(func(w http.ResponseWriter, r *http.Request) {
		resp, _ := spinhttp.Get("https://random-data-api.fermyon.app/animals/json")

		fmt.Fprintln(w, resp.Body)
		fmt.Fprintln(w, resp.Header.Get("content-type"))

		// `spin.toml` is not configured to allow outbound HTTP requests to this host,
		// so this request will fail.
		if _, err := spinhttp.Get("https://fermyon.com"); err != nil {
			fmt.Fprintf(os.Stderr, "Cannot send HTTP request: %v", err)
		}
	})
}
