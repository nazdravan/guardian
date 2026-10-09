import 'package:flutter/material.dart';

import '../theme.dart';

/// What a message reports: colour carries state, so the icon in front of it
/// does too — green done, amber needs attention, red failed.
enum MessageKind { ok, warning, error }

void showSnackBarMessage(
  BuildContext context, {
  required String message,
  MessageKind kind = MessageKind.ok,
  Duration? duration,
}) {
  final (icon, color) = switch (kind) {
    MessageKind.ok => (Icons.check_circle_outline, GuardianColors.guardian),
    MessageKind.warning => (Icons.warning_amber_rounded, GuardianColors.amber),
    MessageKind.error => (Icons.error_outline, GuardianColors.danger),
  };
  ScaffoldMessenger.of(context)
    ..hideCurrentSnackBar()
    ..showSnackBar(
      SnackBar(
        content: Row(
          children: [
            Icon(icon, size: 18, color: color),
            const SizedBox(width: 10),
            Expanded(child: Text(message)),
          ],
        ),
        duration:
            duration ??
            (kind == MessageKind.ok
                ? const Duration(seconds: 2)
                : const Duration(seconds: 4)),
      ),
    );
}
