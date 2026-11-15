package main

import (
	"github.com/lxn/walk"
	. "github.com/lxn/walk/declarative"
)

// colorButton is a push button with a solid colour. Windows draws native
// buttons with the visual style and ignores any background colour, so a
// button that must stand out (red for Shut down) is painted by hand.
type colorButton struct {
	w       *walk.CustomWidget
	text    string
	bg      walk.Color // normal
	down    walk.Color // pressed
	pressed bool
	onClick func()
	font    *walk.Font
}

var (
	dangerBg   = walk.RGB(0xc5, 0x22, 0x1f)
	dangerDown = walk.RGB(0x8c, 0x0b, 0x0b)
	safeBg     = walk.RGB(0x18, 0x80, 0x38)
	safeDown   = walk.RGB(0x0d, 0x5c, 0x26)
	offBg      = walk.RGB(0xb8, 0xbb, 0xbf)
	white      = walk.RGB(0xff, 0xff, 0xff)
)

// widget is the declarative description; size matches a native push button.
func (b *colorButton) widget(tooltip string) Widget {
	return CustomWidget{
		AssignTo:            &b.w,
		MinSize:             Size{Width: 110, Height: 25},
		MaxSize:             Size{Width: 130, Height: 25},
		PaintPixels:         b.paint,
		PaintMode:           PaintBuffered,
		InvalidatesOnResize: true,
		ToolTipText:         tooltip,
		OnMouseDown: func(x, y int, button walk.MouseButton) {
			if button == walk.LeftButton && b.w.Enabled() {
				b.pressed = true
				b.w.Invalidate()
			}
		},
		OnMouseUp: func(x, y int, button walk.MouseButton) {
			if button != walk.LeftButton || !b.pressed {
				return
			}
			b.pressed = false
			b.w.Invalidate()
			// Like a native button: releasing outside it cancels the click.
			if r := b.w.ClientBoundsPixels(); x >= 0 && y >= 0 && x < r.Width && y < r.Height && b.onClick != nil {
				b.onClick()
			}
		},
	}
}

// init finishes what the declarative builder cannot set.
func (b *colorButton) init() {
	b.w.SetCursor(walk.CursorHand())
	b.font, _ = walk.NewFont("Segoe UI", 9, walk.FontBold)
}

func (b *colorButton) Text() string { return b.text }

// set changes the label and colours, repainting only on a real change (the
// window refreshes every 3 s).
func (b *colorButton) set(text string, bg, down walk.Color) {
	if text == b.text && bg == b.bg && down == b.down {
		return
	}
	b.text, b.bg, b.down = text, bg, down
	if b.w != nil {
		b.w.Invalidate()
	}
}

func (b *colorButton) paint(canvas *walk.Canvas, _ walk.Rectangle) error {
	r := b.w.ClientBoundsPixels()

	// The corners show the window behind the rounded rectangle.
	if bgBrush, err := walk.NewSolidColorBrush(windowBg); err == nil {
		canvas.FillRectanglePixels(bgBrush, r)
		bgBrush.Dispose()
	}

	fill := b.bg
	switch {
	case !b.w.Enabled():
		fill = offBg
	case b.pressed:
		fill = b.down
	}
	brush, err := walk.NewSolidColorBrush(fill)
	if err != nil {
		return err
	}
	defer brush.Dispose()
	radius := b.w.IntFrom96DPI(6)
	if err := canvas.FillRoundedRectanglePixels(brush, r, walk.Size{Width: radius, Height: radius}); err != nil {
		return err
	}

	font := b.font
	if font == nil {
		font = b.w.Font()
	}
	return canvas.DrawTextPixels(b.text, font, white, r, walk.TextCenter|walk.TextVCenter|walk.TextSingleLine)
}
