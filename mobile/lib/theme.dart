import 'package:flutter/material.dart';

/// The cabinet's design system on a phone (`specs/cabinet.md`, `server/webui/app.css`).
///
/// Two rules run through it, as in the cabinet:
///
/// * **Type carries who is speaking.** Space Grotesk writes headings, Inter
///   writes the words a person reads, and JetBrains Mono writes what a machine
///   reported — process names, hostnames, agent versions, times from the log.
/// * **Colour carries state and nothing else.** Green is enforcing or online,
///   amber is locked or needs attention, red is destructive or failed. Every
///   other surface is greyscale, so a coloured pixel always means something.
///
/// Dark only, like the cabinet.
abstract final class GuardianColors {
  static const void_ = Color(0xFF07090A);
  static const panel = Color(0xFF0E1412);
  static const raised = Color(0xFF121A17);
  static const sunken = Color(0xFF090D0B);
  static const line = Color(0xFF1E2A24);
  static const lineSoft = Color(0xFF16201B);

  static const guardian = Color(0xFF4ADE80);
  static const signal = Color(0xFF6EFF9B);
  static const amber = Color(0xFFF5C451);
  static const danger = Color(0xFFFF5C6C);

  static const ink = Color(0xFFE6F0EA);
  static const muted = Color(0xFF8FA39A);
  static const dim = Color(0xFF5E6F67);
}

abstract final class GuardianFonts {
  static const ui = 'Inter';
  static const display = 'SpaceGrotesk';
  static const data = 'JetBrainsMono';
}

/// The face for anything a machine reported. Tabular figures, so times and
/// counts line up down a list.
TextStyle monoStyle({
  double size = 13,
  Color color = GuardianColors.ink,
  FontWeight weight = FontWeight.w400,
}) => TextStyle(
  fontFamily: GuardianFonts.data,
  fontSize: size,
  color: color,
  fontWeight: weight,
  fontFeatures: const [FontFeature.tabularFigures()],
);

ThemeData buildGuardianTheme() {
  const scheme = ColorScheme(
    brightness: Brightness.dark,
    primary: GuardianColors.guardian,
    onPrimary: GuardianColors.void_,
    secondary: GuardianColors.muted,
    onSecondary: GuardianColors.void_,
    tertiary: GuardianColors.amber,
    onTertiary: GuardianColors.void_,
    error: GuardianColors.danger,
    onError: GuardianColors.void_,
    surface: GuardianColors.panel,
    onSurface: GuardianColors.ink,
    onSurfaceVariant: GuardianColors.muted,
    surfaceContainerLowest: GuardianColors.sunken,
    surfaceContainerLow: GuardianColors.panel,
    surfaceContainer: GuardianColors.panel,
    surfaceContainerHigh: GuardianColors.raised,
    surfaceContainerHighest: GuardianColors.raised,
    outline: GuardianColors.line,
    outlineVariant: GuardianColors.lineSoft,
  );

  const heading = TextStyle(
    fontFamily: GuardianFonts.display,
    fontWeight: FontWeight.w700,
    letterSpacing: -0.2,
    color: GuardianColors.ink,
  );

  final base = ThemeData(
    useMaterial3: true,
    brightness: Brightness.dark,
    fontFamily: GuardianFonts.ui,
  ).textTheme;

  final text = base
      .copyWith(
        headlineLarge: heading.copyWith(fontSize: 28),
        headlineMedium: heading.copyWith(fontSize: 24),
        headlineSmall: heading.copyWith(fontSize: 20),
        titleLarge: heading.copyWith(fontSize: 20),
        titleMedium: base.titleMedium?.copyWith(fontWeight: FontWeight.w500),
        titleSmall: base.titleSmall?.copyWith(fontWeight: FontWeight.w500),
        labelLarge: base.labelLarge?.copyWith(fontWeight: FontWeight.w600),
      )
      .apply(bodyColor: GuardianColors.ink, displayColor: GuardianColors.ink);

  final rounded = RoundedRectangleBorder(
    borderRadius: BorderRadius.circular(8),
    side: const BorderSide(color: GuardianColors.line),
  );

  return ThemeData(
    useMaterial3: true,
    colorScheme: scheme,
    scaffoldBackgroundColor: GuardianColors.void_,
    fontFamily: GuardianFonts.ui,
    textTheme: text,
    dividerColor: GuardianColors.line,
    dividerTheme: const DividerThemeData(
      color: GuardianColors.line,
      thickness: 1,
      space: 1,
    ),
    appBarTheme: AppBarTheme(
      backgroundColor: GuardianColors.void_,
      foregroundColor: GuardianColors.ink,
      surfaceTintColor: Colors.transparent,
      elevation: 0,
      scrolledUnderElevation: 0,
      centerTitle: false,
      titleTextStyle: heading.copyWith(fontSize: 20),
      shape: const Border(bottom: BorderSide(color: GuardianColors.line)),
    ),
    navigationBarTheme: NavigationBarThemeData(
      backgroundColor: GuardianColors.panel,
      surfaceTintColor: Colors.transparent,
      indicatorColor: GuardianColors.guardian.withValues(alpha: 0.14),
      height: 64,
      labelTextStyle: WidgetStateProperty.resolveWith(
        (states) => TextStyle(
          fontFamily: GuardianFonts.ui,
          fontSize: 12,
          fontWeight: FontWeight.w500,
          color: states.contains(WidgetState.selected)
              ? GuardianColors.ink
              : GuardianColors.muted,
        ),
      ),
      iconTheme: WidgetStateProperty.resolveWith(
        (states) => IconThemeData(
          color: states.contains(WidgetState.selected)
              ? GuardianColors.guardian
              : GuardianColors.muted,
        ),
      ),
    ),
    cardTheme: CardThemeData(
      color: GuardianColors.panel,
      surfaceTintColor: Colors.transparent,
      elevation: 0,
      margin: const EdgeInsets.symmetric(horizontal: 12, vertical: 6),
      shape: rounded,
    ),
    listTileTheme: const ListTileThemeData(
      iconColor: GuardianColors.muted,
      // No textColor: it would paint the subtitle too, and the subtitle is
      // the muted explanation under a setting.
      titleTextStyle: TextStyle(
        fontFamily: GuardianFonts.ui,
        fontSize: 16,
        color: GuardianColors.ink,
      ),
      subtitleTextStyle: TextStyle(
        fontFamily: GuardianFonts.ui,
        fontSize: 13,
        color: GuardianColors.muted,
      ),
    ),
    dialogTheme: DialogThemeData(
      backgroundColor: GuardianColors.panel,
      surfaceTintColor: Colors.transparent,
      shape: rounded,
      titleTextStyle: heading.copyWith(fontSize: 18),
    ),
    bottomSheetTheme: const BottomSheetThemeData(
      backgroundColor: GuardianColors.panel,
      surfaceTintColor: Colors.transparent,
      showDragHandle: true,
      dragHandleColor: GuardianColors.dim,
    ),
    popupMenuTheme: PopupMenuThemeData(
      color: GuardianColors.raised,
      surfaceTintColor: Colors.transparent,
      shape: rounded,
    ),
    snackBarTheme: SnackBarThemeData(
      backgroundColor: GuardianColors.raised,
      contentTextStyle: const TextStyle(
        fontFamily: GuardianFonts.ui,
        color: GuardianColors.ink,
      ),
      behavior: SnackBarBehavior.floating,
      shape: rounded,
    ),
    inputDecorationTheme: InputDecorationTheme(
      filled: true,
      fillColor: GuardianColors.sunken,
      labelStyle: const TextStyle(color: GuardianColors.muted),
      hintStyle: const TextStyle(color: GuardianColors.dim),
      border: OutlineInputBorder(
        borderRadius: BorderRadius.circular(8),
        borderSide: const BorderSide(color: GuardianColors.line),
      ),
      enabledBorder: OutlineInputBorder(
        borderRadius: BorderRadius.circular(8),
        borderSide: const BorderSide(color: GuardianColors.line),
      ),
      focusedBorder: OutlineInputBorder(
        borderRadius: BorderRadius.circular(8),
        borderSide: const BorderSide(color: GuardianColors.signal, width: 1.5),
      ),
    ),
    filledButtonTheme: FilledButtonThemeData(
      style: FilledButton.styleFrom(
        backgroundColor: GuardianColors.guardian,
        foregroundColor: GuardianColors.void_,
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(8)),
        textStyle: const TextStyle(
          fontFamily: GuardianFonts.ui,
          fontWeight: FontWeight.w600,
        ),
      ),
    ),
    outlinedButtonTheme: OutlinedButtonThemeData(
      style: OutlinedButton.styleFrom(
        foregroundColor: GuardianColors.ink,
        side: const BorderSide(color: GuardianColors.line),
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(8)),
      ),
    ),
    textButtonTheme: TextButtonThemeData(
      style: TextButton.styleFrom(foregroundColor: GuardianColors.guardian),
    ),
    floatingActionButtonTheme: const FloatingActionButtonThemeData(
      backgroundColor: GuardianColors.guardian,
      foregroundColor: GuardianColors.void_,
      elevation: 0,
    ),
    chipTheme: ChipThemeData(
      backgroundColor: GuardianColors.panel,
      selectedColor: GuardianColors.guardian.withValues(alpha: 0.14),
      side: const BorderSide(color: GuardianColors.line),
      labelStyle: const TextStyle(
        fontFamily: GuardianFonts.ui,
        fontSize: 13,
        color: GuardianColors.ink,
      ),
      checkmarkColor: GuardianColors.guardian,
      shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(6)),
    ),
    switchTheme: SwitchThemeData(
      thumbColor: WidgetStateProperty.resolveWith(
        (s) => s.contains(WidgetState.selected)
            ? GuardianColors.void_
            : GuardianColors.muted,
      ),
      trackColor: WidgetStateProperty.resolveWith(
        (s) => s.contains(WidgetState.selected)
            ? GuardianColors.guardian
            : GuardianColors.sunken,
      ),
      trackOutlineColor: WidgetStateProperty.resolveWith(
        (s) => s.contains(WidgetState.selected)
            ? GuardianColors.guardian
            : GuardianColors.line,
      ),
    ),
    segmentedButtonTheme: SegmentedButtonThemeData(
      style: SegmentedButton.styleFrom(
        backgroundColor: GuardianColors.sunken,
        foregroundColor: GuardianColors.muted,
        selectedBackgroundColor: GuardianColors.guardian.withValues(
          alpha: 0.14,
        ),
        selectedForegroundColor: GuardianColors.ink,
        side: const BorderSide(color: GuardianColors.line),
      ),
    ),
    progressIndicatorTheme: const ProgressIndicatorThemeData(
      color: GuardianColors.guardian,
    ),
  );
}
