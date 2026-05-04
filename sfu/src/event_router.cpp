#include "event_router.h"

#include "absl/log/log.h"

void EventRouter::registerSignaling(const std::string& signalingId,
                                    const std::shared_ptr<EventRouter::Subscriber>& sub) {
    std::deque<sfu::SFUEvent> pendingBatch;
    std::shared_ptr<EventRouter::Subscriber> subCopy;
    {
        std::lock_guard lock(m_mutex);
        m_subscribers[signalingId] = sub;
        subCopy = sub;
        auto pit = m_pending.find(signalingId);
        if (pit != m_pending.end()) {
            pendingBatch = std::move(pit->second);
            m_pending.erase(pit);
        }
    }
    for (const auto& ev : pendingBatch) {
        std::lock_guard wlock(subCopy->m_writeMutex);
        if (subCopy->m_writer != nullptr) {
            if (!subCopy->m_writer->Write(ev)) {
                ABSL_LOG(WARNING) << "Failed to flush pending SFU event for signaling_id="
                                  << signalingId;
                break;
            }
        }
    }
}

void EventRouter::unregisterSignaling(const std::string& signalingId,
                                      const std::shared_ptr<EventRouter::Subscriber>& sub) {
    std::lock_guard lock(m_mutex);
    auto it = m_subscribers.find(signalingId);
    if (it != m_subscribers.end() && it->second == sub) {
        m_subscribers.erase(it);
    }
}

bool EventRouter::routeEvent(const std::string& signalingId, const sfu::SFUEvent& event) {
    std::shared_ptr<EventRouter::Subscriber> sub;
    {
        std::lock_guard lock(m_mutex);
        auto it = m_subscribers.find(signalingId);
        if (it == m_subscribers.end()) {
            auto& q = m_pending[signalingId];
            q.push_back(event);
            while (q.size() > maxPendingEvents) {
                ABSL_LOG(WARNING) << "Dropping oldest pending SFU event due to queue overflow"
                                  << " signaling_id=" << signalingId
                                  << " max_pending=" << maxPendingEvents;
                q.pop_front();
            }
            return true;
        }
        sub = it->second;
    }
    std::lock_guard wlock(sub->m_writeMutex);
    if (sub->m_writer == nullptr) {
        return false;
    }
    return sub->m_writer->Write(event);
}
