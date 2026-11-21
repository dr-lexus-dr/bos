// The browser engine runs inside a bOS WebAssembly instance. Its platform services are
// input events, a bounded HTTP transport and presentation to its own window.
package main

import (
	"bytes"
	"compress/gzip"
	"compress/zlib"
	"context"
	"encoding/binary"
	"fmt"
	"image"
	"io"
	"net/http"
	"net/http/cookiejar"
	"net/url"
	"runtime/debug"
	"strings"
	"time"
	"unsafe"

	"github.com/andybalholm/brotli"
	"github.com/go-webengine/engine"
	"github.com/go-webengine/engine/dom"
	"golang.org/x/net/publicsuffix"
)

//go:wasmimport bos event
func event(p unsafe.Pointer, n uint32) int32

//go:wasmimport bos request
func request(meta unsafe.Pointer, n uint32, body unsafe.Pointer, size uint32) int32

//go:wasmimport bos response
func response(p unsafe.Pointer, n uint32, part uint32) int32

//go:wasmimport bos present
func present(p unsafe.Pointer, width, height, documentHeight, scroll uint32, meta unsafe.Pointer, size uint32)

//go:wasmimport bos notice
func notice(p unsafe.Pointer, n uint32)

func notify(s string)             { b := []byte(s); notice(unsafe.Pointer(unsafe.SliceData(b)), uint32(len(b))) }
func ptr(b []byte) unsafe.Pointer { return unsafe.Pointer(unsafe.SliceData(b)) }

type transport struct{}

func (transport) RoundTrip(r *http.Request) (*http.Response, error) {
	if r.URL.Scheme != "http" && r.URL.Scheme != "https" {
		return nil, fmt.Errorf("unsupported protocol %q", r.URL.Scheme)
	}
	var body []byte
	if r.Body != nil {
		defer r.Body.Close()
		var err error
		body, err = io.ReadAll(io.LimitReader(r.Body, (1<<20)+1))
		if err != nil || len(body) > 1<<20 {
			return nil, fmt.Errorf("request body is too large")
		}
	}
	var header strings.Builder
	fmt.Fprintf(&header, "%s\n%s\n", r.Method, r.URL.String())
	for _, key := range []string{"Content-Type", "Cookie", "Accept", "Origin", "Referer", "Authorization", "If-None-Match"} {
		if v := r.Header.Get(key); v != "" && !strings.ContainsAny(v, "\r\n") {
			fmt.Fprintf(&header, "%s: %s\n", key, v)
		}
	}
	meta := []byte(header.String())
	size := request(ptr(meta), uint32(len(meta)), ptr(body), uint32(len(body)))
	var info [16384]byte
	k := response(unsafe.Pointer(&info[0]), uint32(len(info)), 0)
	if k < 0 || int(k) > len(info) {
		return nil, fmt.Errorf("invalid response metadata")
	}
	raw := string(info[:k])
	if size < 0 {
		return nil, fmt.Errorf("%s", raw)
	}
	if size > 16<<20 {
		return nil, fmt.Errorf("resource exceeds memory limit")
	}
	data := make([]byte, size)
	if response(ptr(data), uint32(size), 1) != size {
		return nil, fmt.Errorf("incomplete response")
	}
	lines := strings.Split(raw, "\n")
	var status int
	if _, err := fmt.Sscan(lines[0], &status); err != nil || status < 100 || status > 599 {
		return nil, fmt.Errorf("invalid HTTP status")
	}
	h := make(http.Header)
	for _, line := range lines[1:] {
		if k, v, ok := strings.Cut(line, ":"); ok {
			h.Add(k, strings.TrimSpace(v))
		}
	}
	var reader io.Reader = bytes.NewReader(data)
	switch strings.ToLower(h.Get("Content-Encoding")) {
	case "gzip":
		z, err := gzip.NewReader(reader)
		if err != nil {
			return nil, err
		}
		defer z.Close()
		reader = z
	case "deflate":
		z, err := zlib.NewReader(reader)
		if err != nil {
			return nil, err
		}
		defer z.Close()
		reader = z
	case "br":
		reader = brotli.NewReader(reader)
	case "", "identity":
	default:
		return nil, fmt.Errorf("unsupported content encoding")
	}
	if h.Get("Content-Encoding") != "" {
		plain, err := io.ReadAll(io.LimitReader(reader, (16<<20)+1))
		if err != nil || len(plain) > 16<<20 {
			return nil, fmt.Errorf("unable to decode response")
		}
		data = plain
		h.Del("Content-Encoding")
		h.Del("Content-Length")
	}
	return &http.Response{StatusCode: status, Status: fmt.Sprintf("%d %s", status, http.StatusText(status)), Header: h, Body: io.NopCloser(bytes.NewReader(data)), ContentLength: int64(len(data)), Request: r}, nil
}

type window struct {
	browser               *engine.Engine
	doc                   *engine.LiveDocument
	field                 *dom.Node
	width, height, scroll int
	status                int
	cache                 resourceCache
}

func (w *window) draw(img *image.RGBA, info *engine.RenderInfo, err error) {
	if err != nil {
		notify(err.Error())
		return
	}
	if img == nil || info == nil || w.doc == nil || img.Rect.Empty() {
		return
	}
	w.scroll = w.doc.ScrollOffset()
	doc := w.doc.Document()
	meta := []byte(fmt.Sprintf("%d\n%s\nready\n%s", w.status, doc.URL, doc.Title))
	documentHeight := min(max(info.ContentHeight, img.Rect.Dy()), 1<<30)
	present(ptr(img.Pix), uint32(img.Rect.Dx()), uint32(img.Rect.Dy()), uint32(documentHeight), uint32(w.scroll), ptr(meta), uint32(len(meta)))
}
func (w *window) open(ctx context.Context, address string) {
	w.status = 0
	page, err := w.browser.Fetch(ctx, address)
	if err != nil {
		notify(err.Error())
		return
	}
	status := w.status
	d := w.browser.OpenDesktopPage(ctx, page, image.Rect(0, 0, w.width, w.height), func(img *image.RGBA) {
		if img.Rect.Empty() {
			return
		}
		meta := []byte(fmt.Sprintf("%d\n%s\npreview\n%s", status, page.URL, page.Title))
		present(ptr(img.Pix), uint32(img.Rect.Dx()), uint32(img.Rect.Dy()), uint32(img.Rect.Dy()), 0, ptr(meta), uint32(len(meta)))
	})
	w.status = status
	if w.doc != nil {
		w.doc.Close()
	}
	w.doc, w.field, w.scroll = d, nil, 0
	w.draw(d.Frame())
}
func (w *window) submit(ctx context.Context, n *dom.Node) {
	for ; n != nil; n = n.Parent {
		if n.Tag == "form" {
			status := w.status
			w.status = 0
			d, changed, err := w.doc.Submit(ctx, n)
			if !changed {
				w.status = status
			}
			if err != nil {
				notify(err.Error())
				return
			}
			if changed {
				w.doc, w.field, w.scroll = d, nil, 0
			}
			w.draw(w.doc.Frame())
			return
		}
	}
	w.draw(w.doc.Frame())
}
func (w *window) activate(ctx context.Context, n *dom.Node) {
	for ; n != nil; n = n.Parent {
		switch n.Tag {
		case "input", "textarea", "button", "a", "select", "label":
			if _, disabled := n.Attribute("disabled"); disabled {
				return
			}
			w.field = n
			stopped, img, info, err := w.doc.Click(ctx, n)
			if err != nil {
				notify(err.Error())
				return
			}
			if stopped {
				w.draw(img, info, nil)
				return
			}
			typ := strings.ToLower(n.Attr["type"])
			if n.Tag == "a" && n.Attr["href"] != "" {
				base, _ := url.Parse(w.doc.Document().URL)
				ref, e := url.Parse(n.Attr["href"])
				if e == nil && base != nil {
					addr := base.ResolveReference(ref)
					if strings.HasPrefix(n.Attr["href"], "#") {
						for el, box := range w.doc.Elements() {
							if el.ID() == addr.Fragment || el.Tag == "a" && el.Attr["name"] == addr.Fragment {
								w.scroll = int(box.Y)
								break
							}
						}
						if addr.Fragment == "" {
							w.scroll = 0
						}
						w.doc.Document().URL = addr.String()
						w.draw(w.doc.Scroll(w.scroll))
					} else {
						w.open(ctx, addr.String())
					}
				}
				return
			}
			if n.Tag == "button" && (typ == "" || typ == "submit") || n.Tag == "input" && typ == "submit" {
				w.submit(ctx, n)
				return
			}
			if n.Tag == "input" && (typ == "checkbox" || typ == "radio") || n.Tag == "select" {
				w.draw(w.doc.ChangeControl(ctx, n, 1))
			} else if n.Tag == "label" && n.Attr["for"] != "" {
				for _, field := range w.doc.QuerySelectorAll("input, select, textarea") {
					if field.ID() == n.Attr["for"] {
						w.activate(ctx, field)
						return
					}
				}
				w.draw(img, info, nil)
			} else {
				w.draw(img, info, nil)
			}
			return
		}
		// An element can have a click listener without being a form control.
		if n.Attr["onclick"] != "" {
			break
		}
	}
}
func (w *window) interact(ctx context.Context, kind, a, b int, s string) {
	if w.doc == nil {
		return
	}
	w.doc.BeginInput()
	switch kind {
	case 2:
		if n, ok := w.doc.ElementAt(image.Pt(a, b+w.scroll)); ok {
			target := n
			for target != nil && target.Tag != "a" && target.Tag != "input" && target.Tag != "textarea" && target.Tag != "button" && target.Tag != "select" && target.Tag != "label" {
				target = target.Parent
			}
			if target != nil {
				w.activate(ctx, target)
			} else {
				_, img, i, e := w.doc.Click(ctx, n)
				w.draw(img, i, e)
			}
		}
	case 3:
		if w.field != nil {
			typ := strings.ToLower(w.field.Attr["type"])
			if s == " " && (w.field.Tag == "button" || w.field.Tag == "input" && (typ == "checkbox" || typ == "radio")) {
				w.activate(ctx, w.field)
			} else if w.field.Tag == "textarea" || w.field.Tag == "input" && typ != "checkbox" && typ != "radio" && typ != "submit" && typ != "button" {
				w.draw(w.doc.Type(ctx, w.field, s))
			}
		}
	case 4:
		if w.field != nil && w.field.Tag == "select" && (a == 34 || a == -34) {
			w.draw(w.doc.ChangeControl(ctx, w.field, a/34))
			return
		}
		w.draw(w.doc.Scroll(w.scroll + a))
	case 5:
		if w.field != nil {
			w.draw(w.doc.Backspace(ctx, w.field))
		}
	case 6:
		if w.field != nil {
			stop, img, i, e := w.doc.KeyDown(ctx, w.field, "Enter")
			if stop || e != nil {
				w.draw(img, i, e)
			} else if w.field.Tag == "a" || w.field.Tag == "button" {
				w.activate(ctx, w.field)
			} else {
				w.submit(ctx, w.field)
			}
		}
	case 8:
		if a > 0 && b > 0 {
			w.width, w.height = a, b
			w.draw(w.doc.Resize(ctx, image.Rect(0, 0, a, b)))
		}
	case 7:
		tabbable := w.doc.QuerySelectorAll("a[href], input, textarea, button, select")
		if len(tabbable) > 0 {
			index := 0
			for i, n := range tabbable {
				if n == w.field {
					index = (i + 1) % len(tabbable)
					break
				}
			}
			w.field = tabbable[index]
			w.draw(w.doc.Focus(ctx, w.field))
		}
	}
}

type withStatus struct{ w *window }

func (t withStatus) RoundTrip(r *http.Request) (*http.Response, error) {
	out, err := t.w.cache.RoundTrip(r)
	if out != nil && t.w.status == 0 {
		t.w.status = out.StatusCode
	}
	return out, err
}
func main() {
	debug.SetMemoryLimit(192 << 20)
	e := engine.New()
	w := window{browser: e, width: 800, height: 600}
	jar, _ := cookiejar.New(&cookiejar.Options{PublicSuffixList: publicsuffix.List})
	e.Client = &http.Client{Transport: withStatus{&w}, Jar: jar, CheckRedirect: func(req *http.Request, via []*http.Request) error {
		if len(via) >= 8 {
			return fmt.Errorf("too many redirects")
		}
		if len(via) > 0 && via[len(via)-1].URL.Scheme == "https" && req.URL.Scheme != "https" {
			return fmt.Errorf("HTTPS downgrade blocked")
		}
		w.status = 0
		return nil
	}}
	e.JSTimeout = 12 * time.Second
	e.MaxJSHeapBytes = 40 << 20
	e.MaxImages = 40
	e.MaxRequests = 96
	ctx := context.Background()
	var packet [8192]byte
	for {
		n := event(unsafe.Pointer(&packet[0]), uint32(len(packet)))
		if n < 12 {
			break
		}
		kind := int(binary.LittleEndian.Uint32(packet[:4]))
		a := int(int32(binary.LittleEndian.Uint32(packet[4:8])))
		b := int(int32(binary.LittleEndian.Uint32(packet[8:12])))
		s := string(packet[12:n])
		if kind == 1 {
			w.width, w.height = max(80, a), max(60, b)
			w.status = 0
			w.open(ctx, s)
		} else {
			w.interact(ctx, kind, a, b, s)
		}
	}
	if w.doc != nil {
		w.doc.Close()
	}
}
