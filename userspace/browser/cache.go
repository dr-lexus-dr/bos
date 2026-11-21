package main

import (
	"bytes"
	"io"
	"net/http"
	"strconv"
	"strings"
	"time"
)

// Keep explicitly fresh, anonymous GET responses for repeated layout and paint
// in one browser window. Authenticated and varying responses always use the wire.
type cachedResource struct {
	key    string
	until  time.Time
	header http.Header
	status string
	data   []byte
}

type resourceCache struct {
	entries []cachedResource // least recently used first
	size    int
}

func cacheTTL(h http.Header) time.Duration {
	if h.Get("Set-Cookie") != "" {
		return 0
	}
	for _, vary := range h.Values("Vary") {
		for _, key := range strings.Split(strings.ToLower(vary), ",") {
			switch strings.TrimSpace(key) {
			case "", "accept", "accept-encoding":
			default:
				return 0
			}
		}
	}
	var age int64
	for _, value := range h.Values("Cache-Control") {
		for _, raw := range strings.Split(strings.ToLower(value), ",") {
			name, value, _ := strings.Cut(strings.TrimSpace(raw), "=")
			switch name {
			case "private", "no-store", "no-cache":
				return 0
			case "max-age":
				age, _ = strconv.ParseInt(strings.Trim(value, "\""), 10, 32)
			}
		}
	}
	used, _ := strconv.ParseInt(h.Get("Age"), 10, 32)
	return time.Duration(min(3600, max(0, age-max(used, 0)))) * time.Second
}

func (c *resourceCache) RoundTrip(r *http.Request) (*http.Response, error) {
	if r.Method != "GET" || r.Header.Get("Cookie") != "" || r.Header.Get("Authorization") != "" || r.Header.Get("Cache-Control") != "" {
		return (transport{}).RoundTrip(r)
	}
	key := r.URL.String() + "\n" + r.Header.Get("Accept") + "\n" + r.Header.Get("Origin")
	now := time.Now()
	for i, entry := range c.entries {
		if entry.key != key {
			continue
		}
		copy(c.entries[i:], c.entries[i+1:])
		if now.Before(entry.until) {
			c.entries[len(c.entries)-1] = entry
			return &http.Response{StatusCode: 200, Status: entry.status, Header: entry.header.Clone(), Body: io.NopCloser(bytes.NewReader(entry.data)), ContentLength: int64(len(entry.data)), Request: r}, nil
		}
		c.entries[len(c.entries)-1] = cachedResource{}
		c.entries = c.entries[:len(c.entries)-1]
		c.size -= len(entry.data)
		break
	}
	out, err := (transport{}).RoundTrip(r)
	if err != nil || out.StatusCode != 200 || out.ContentLength < 0 || out.ContentLength > 512<<10 {
		return out, err
	}
	ttl := cacheTTL(out.Header)
	if ttl <= 0 {
		return out, nil
	}
	data, err := io.ReadAll(out.Body)
	out.Body.Close()
	if err != nil {
		return nil, err
	}
	out.Body = io.NopCloser(bytes.NewReader(data))
	for len(c.entries) >= 24 || c.size+len(data) > 4<<20 {
		c.size -= len(c.entries[0].data)
		c.entries[0] = cachedResource{}
		c.entries = c.entries[1:]
	}
	c.entries = append(c.entries, cachedResource{key: key, until: now.Add(ttl), header: out.Header.Clone(), status: out.Status, data: data})
	c.size += len(data)
	return out, nil
}
