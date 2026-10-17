# Glossary

**Area**
The rectangle of the real screen that viewers see. Freely shaped but always an even number of pixels wide and tall, defaults to 2560×1440 centered. The stream is the Area pixel for pixel, never scaled or padded.
_Avoid_: region, crop, zone

**Border**
The outline drawn around the Area, with the rest of the screen shaded, so its owner can see what viewers see. Never visible to viewers.
_Avoid_: overlay, frame, highlight

**Edit mode**
The state while the hotkey (Ctrl+Alt) is held: the Border shows and the Area can be resized or moved with the mouse. Releasing the hotkey commits the change.

**Virtual monitor**
The extra screen cropcast adds to Windows, exactly the size of the Area, which carries the Area's picture. It's what Discord's "Screen" share is pointed at. It's also cropcast's on/off state, toggled from the tray: while the Virtual monitor is on, it exists and mirrors the Area; while it's off, it doesn't exist and the desktop is untouched.
_Avoid_: fake display, second screen, virtual desktop (Windows' Win+Tab desktops), Sharing (that's Discord's act, not cropcast's)

**Driver**
The virtual display driver cropcast installs, once, so the Virtual monitor can exist. Installing or removing it is the only thing cropcast asks admin rights for.
_Avoid_: VDA, display adapter
