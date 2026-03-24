@0xb3a2c1d4e5f60718;

struct Command {
  id @0 :UInt32;
  union {
    status @1 :Void;
    toggle @2 :Text;
    reload @3 :Void;
    spike @4 :Void;
    subscribe @5 :Void;
  }
}

struct CommandResponse {
  id @0 :UInt32;
  union {
    ok @1 :Void;
    error @2 :Text;
  }
}

struct StateSnapshot {
  uptimeSecs @0 :UInt64;
  potential @1 :Float64;
  threshold @2 :Float64;
  powerState @3 :PowerState;
  eventsPerMin @4 :Float32;
  connectors @5 :List(ConnectorState);
  framerate @6 :UInt8;
}

enum PowerState {
  active @0;
  resting @1;
}

struct ConnectorState {
  name @0 :Text;
  status @1 :Status;
  sparkline @2 :List(UInt8);
  eventCount @3 :UInt64;
  errorMessage @4 :Text;
}

enum Status {
  running @0;
  stopped @1;
  suspended @2;
  error @3;
}

struct ActivityEvent {
  timestampMs @0 :UInt64;
  source @1 :Text;
  eventType @2 :Text;
  detail @3 :Text;
}

struct DaemonMessage {
  union {
    state @0 :StateSnapshot;
    activity @1 :ActivityEvent;
    power @2 :PowerState;
    response @3 :CommandResponse;
  }
}
