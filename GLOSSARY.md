# Glossary

**Area**
The rectangle of the real screen that viewers see. Freely shaped, defaults to 2560×1440 centered. The stream is the Area pixel for pixel, never scaled or padded.
_Avoid_: region, crop, zone

**Border**
The outline drawn around the Area so its owner can see what viewers see. Never visible to viewers.
_Avoid_: overlay, frame, highlight

**Edit mode**
The state while the hotkey (Ctrl+Alt) is held: the Border shows and the Area can be resized or moved with the mouse. Releasing the hotkey commits the change.

**Virtual monitor**
The extra screen cropcast adds to Windows, exactly the size of the Area, which carries the Area's picture. It's what Discord's "Screen" share is pointed at.
_Avoid_: fake display, second screen

**Sharing**
The on/off state of cropcast. While Sharing is on, the Virtual monitor exists and mirrors the Area. While it's off, the Virtual monitor doesn't exist and the desktop is untouched.
