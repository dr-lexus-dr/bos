// Platform adapter for script-initiated fetch and XMLHttpRequest in bOS.
package engine

import (
	"fmt"
	"mime"
	"net"
	"net/http"
	"net/url"
	"strings"
)

func origin(u *url.URL) string {
	host := strings.ToLower(u.Hostname())
	port := u.Port()
	if (u.Scheme == "http" && port == "80") || (u.Scheme == "https" && port == "443") {
		port = ""
	}
	if port != "" {
		host = net.JoinHostPort(host, port)
	} else if strings.Contains(host, ":") {
		host = "[" + host + "]"
	}
	return u.Scheme + "://" + host
}

type pageTransport struct {
	site   string
	client *http.Client
}

func (e *Engine) desktopScriptClient(address string) *http.Client {
	u, err := url.Parse(address)
	if err != nil || e.Client == nil {
		return nil
	}
	return &http.Client{Transport: pageTransport{origin(u), e.Client}, CheckRedirect: e.Client.CheckRedirect, Timeout: e.Client.Timeout}
}

func (t pageTransport) RoundTrip(r *http.Request) (*http.Response, error) {
	cross := origin(r.URL) != t.site
	if strings.HasPrefix(t.site, "https://") && r.URL.Scheme != "https" {
		return nil, fmt.Errorf("mixed content blocked")
	}
	if cross && r.Method != "GET" && r.Method != "HEAD" && r.Method != "POST" {
		return nil, fmt.Errorf("cross-origin method requires preflight")
	}
	req := r.Clone(r.Context())
	for _, key := range []string{"Cookie", "Cookie2", "Origin", "Referer", "Host", "Proxy-Authorization"} {
		req.Header.Del(key)
	}
	if cross {
		for key := range req.Header {
			switch strings.ToLower(key) {
			case "accept", "accept-language", "content-language", "user-agent":
			case "content-type":
				m, _, err := mime.ParseMediaType(req.Header.Get(key))
				if err != nil || (m != "application/x-www-form-urlencoded" && m != "multipart/form-data" && m != "text/plain") {
					return nil, fmt.Errorf("cross-origin content type requires preflight")
				}
			default:
				return nil, fmt.Errorf("cross-origin header requires preflight")
			}
		}
		req.Header.Set("Origin", t.site)
	} else if t.client.Jar != nil {
		for _, c := range t.client.Jar.Cookies(req.URL) {
			req.AddCookie(c)
		}
	}
	resp, err := t.client.Transport.RoundTrip(req)
	if err != nil {
		return nil, err
	}
	if cross {
		allow := resp.Header.Get("Access-Control-Allow-Origin")
		if allow != "*" && allow != t.site {
			resp.Body.Close()
			return nil, fmt.Errorf("cross-origin response blocked")
		}
	} else if t.client.Jar != nil {
		t.client.Jar.SetCookies(req.URL, resp.Cookies())
	}
	resp.Header.Del("Set-Cookie")
	resp.Header.Del("Set-Cookie2")
	return resp, nil
}
