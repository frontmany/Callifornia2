#pragma once

#include "signaling.grpc.pb.h"

#include <grpcpp/grpcpp.h>

#include <cstddef>
#include <deque>
#include <memory>
#include <mutex>
#include <string>
#include <unordered_map>

// Routes SFUEvent to the SubscribeEvents stream for a given signaling instance.
// Buffers a small backlog if events arrive before the signaling stream registers (ordering with CreatePeer).
class EventRouter {
public:
    struct Subscriber {
        std::mutex m_writeMutex;
        grpc::ServerWriter<sfu::SFUEvent>* m_writer = nullptr;
    };

    void registerSignaling(const std::string& signalingId, const std::shared_ptr<Subscriber>& sub);
    void unregisterSignaling(const std::string& signalingId, const std::shared_ptr<Subscriber>& sub);

    // Returns true if the event was delivered or queued; false only if dropped or write failed.
    bool routeEvent(const std::string& signalingId, const sfu::SFUEvent& event);

private:
    static constexpr std::size_t maxPendingEvents = 128;

    mutable std::mutex m_mutex;
    std::unordered_map<std::string, std::shared_ptr<Subscriber>> m_subscribers;
    std::unordered_map<std::string, std::deque<sfu::SFUEvent>> m_pending;
};
