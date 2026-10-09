import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:procsentinel_mobile/services/settings_service.dart';
import 'package:shared_preferences/shared_preferences.dart';

/// Records what the app sent and replies with whatever the test wants.
class _Recorder {
  final List<http.Request> requests = [];
  late MockClient client;

  _Recorder(Map<String, http.Response> Function(http.Request) respond) {
    client = MockClient((request) async {
      requests.add(request);
      final key = '${request.method} ${request.url.path}';
      final replies = respond(request);
      final reply = replies[key];
      if (reply == null) {
        return http.Response(json.encode({'error': 'unexpected $key'}), 500);
      }
      return reply;
    });
  }

  http.Request get last => requests.last;
}

SettingsService _serviceWith(_Recorder recorder) {
  final service = SettingsService();
  service.resetForTest();
  service.clientFactory = () => recorder.client;
  return service;
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  setUp(() => SharedPreferences.setMockInitialValues({}));

  test('signing in asks for a token in the body and stores it', () async {
    final recorder = _Recorder(
      (_) => {
        'POST /api/v1/auth/login': http.Response(
          json.encode({'token': 'session-abc'}),
          200,
        ),
        'GET /api/v1/me': http.Response(
          json.encode({
            'user_id': 'u1',
            'accounts': [
              {'account_id': 'acc-1', 'name': 'Home', 'role': 'owner'},
            ],
          }),
          200,
        ),
      },
    );
    final service = _serviceWith(recorder);
    await service.load();

    await service.signIn('parent@example.com', 'a-long-enough-password');

    final login = recorder.requests.first;
    final sent = json.decode(login.body) as Map<String, dynamic>;
    // A browser deliberately gets the token only in an HttpOnly cookie, so the
    // app has to say it is not one.
    expect(sent['client'], 'mobile');
    expect(service.isSignedIn, isTrue);
    // Landing in an account is part of signing in: a user who is a guest
    // somewhere would otherwise see an empty screen and no way out of it.
    expect(service.accountId, 'acc-1');
  });

  test(
    'authenticated requests carry the session and the chosen account',
    () async {
      final recorder = _Recorder(
        (_) => {
          'GET /api/v1/rooms': http.Response(json.encode({'rooms': []}), 200),
        },
      );
      final service = _serviceWith(recorder);
      await service.load();
      await service.signInForTest('session-abc', 'acc-7');

      await service.rooms();

      expect(recorder.last.headers['Authorization'], 'Bearer session-abc');
      expect(recorder.last.headers['X-Guardian-Account'], 'acc-7');
    },
  );

  test('the server error message is what the user is shown', () async {
    final recorder = _Recorder(
      (_) => {
        'POST /api/v1/rooms': http.Response(
          json.encode({'error': 'Only account admins can do that'}),
          403,
        ),
      },
    );
    final service = _serviceWith(recorder);
    await service.load();
    await service.signInForTest('session-abc', 'acc-7');

    await expectLater(
      service.createRoom('Study'),
      throwsA(
        isA<ApiException>()
            .having(
              (e) => e.message,
              'message',
              'Only account admins can do that',
            )
            .having((e) => e.isForbidden, 'isForbidden', isTrue),
      ),
    );
  });

  test('an unreachable server is not the same as an empty account', () async {
    final service = SettingsService();
    service.resetForTest();
    service.clientFactory = () =>
        MockClient((_) async => throw const SocketExceptionStub());
    await service.load();
    await service.signInForTest('session-abc', 'acc-7');

    // The old client returned [] here, so an outage looked exactly like "you
    // have no rooms" — the screen said the account was empty.
    await expectLater(service.rooms(), throwsA(isA<ApiException>()));
  });

  test(
    'switching accounts forgets the room, which belonged to the old one',
    () async {
      final recorder = _Recorder((_) => {});
      final service = _serviceWith(recorder);
      await service.load();
      await service.setAccountId('acc-1');
      await service.setRoomId('room-1');
      expect(service.roomId, 'room-1');

      await service.setAccountId('acc-2');
      expect(service.roomId, isNull);
    },
  );

  group('blocking log', () {
    final page = {
      'process_events': [
        {
          'id': 'e1',
          'computer_id': 'c1',
          'computer_name': 'LAB-A-01',
          'room_id': 'room-1',
          'process': 'steam.exe',
          'reason': 'whitelist',
          'count': 47,
          'first_at': '2026-10-09T14:00:00Z',
          'last_at': '2026-10-09T14:00:46Z',
          'created_at': '2026-10-09T14:01:03Z',
        },
      ],
      'next_cursor': 'cur-2',
    };

    test('the room log sends only the filters that are set', () async {
      final recorder = _Recorder(
        (_) => {
          'GET /api/v1/rooms/room-1/process-events': http.Response(
            json.encode(page),
            200,
          ),
        },
      );
      final service = _serviceWith(recorder);
      await service.load();
      await service.signInForTest('session-abc', 'acc-7');

      final result = await service.roomProcessEvents(
        'room-1',
        filter: ProcessEventFilter(
          process: '  steam.exe ',
          reason: 'whitelist',
          computerId: 'c1',
          since: DateTime.utc(2026, 10, 8, 12),
        ),
      );

      final q = recorder.last.url.queryParameters;
      expect(q['process'], 'steam.exe');
      expect(q['reason'], 'whitelist');
      expect(q['computer_id'], 'c1');
      // The server parses RFC3339; a local time without an offset would be
      // silently ignored as a malformed filter.
      expect(DateTime.parse(q['since']!), DateTime.utc(2026, 10, 8, 12));
      expect(q.containsKey('cursor'), isFalse);
      expect(recorder.last.headers['X-Guardian-Account'], 'acc-7');

      expect(result.events.single['process'], 'steam.exe');
      expect(result.nextCursor, 'cur-2');
    });

    test('an empty filter is a bare request', () async {
      final recorder = _Recorder(
        (_) => {
          'GET /api/v1/rooms/room-1/process-events': http.Response(
            json.encode({'process_events': []}),
            200,
          ),
        },
      );
      final service = _serviceWith(recorder);
      await service.load();
      await service.signInForTest('session-abc', 'acc-7');

      final result = await service.roomProcessEvents('room-1');

      expect(recorder.last.url.query, isEmpty);
      expect(result.events, isEmpty);
      // No next_cursor is how the API says "that was the last page".
      expect(result.nextCursor, isNull);
    });

    test('a computer log pages by cursor and has no computer filter', () async {
      final recorder = _Recorder(
        (_) => {
          'GET /api/v1/computers/c1/process-events': http.Response(
            json.encode(page),
            200,
          ),
        },
      );
      final service = _serviceWith(recorder);
      await service.load();
      await service.signInForTest('session-abc', 'acc-7');

      await service.computerProcessEvents(
        'c1',
        filter: const ProcessEventFilter(computerId: 'c9'),
        cursor: 'cur-2',
      );

      final q = recorder.last.url.queryParameters;
      expect(q['cursor'], 'cur-2');
      expect(q.containsKey('computer_id'), isFalse);
    });

    test('a broken cursor surfaces as the server\'s error', () async {
      final recorder = _Recorder(
        (_) => {
          'GET /api/v1/rooms/room-1/process-events': http.Response(
            json.encode({'error': 'Invalid cursor'}),
            400,
          ),
        },
      );
      final service = _serviceWith(recorder);
      await service.load();
      await service.signInForTest('session-abc', 'acc-7');

      await expectLater(
        service.roomProcessEvents('room-1', cursor: 'garbage'),
        throwsA(
          isA<ApiException>().having(
            (e) => e.message,
            'message',
            'Invalid cursor',
          ),
        ),
      );
    });
  });
}

/// A stand-in for a transport failure; MockClient has no built-in way to throw
/// a socket error and the type does not matter, only that it is not an HTTP
/// response.
class SocketExceptionStub implements Exception {
  const SocketExceptionStub();
}
