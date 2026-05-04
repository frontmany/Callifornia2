#include "peer_session.h"

#include "event_router.h"

#include "signaling.pb.h"

#include <chrono>
#include <future>
#include <stdexcept>
#include <utility>

PeerSession::PeerSession(std::string roomId, std::string participantId, std::string signalingId)
    : m_roomId(std::move(roomId)),
      m_participantId(std::move(participantId)),
      m_signalingId(std::move(signalingId)) {}

PeerSession::~PeerSession() {
    shutdownPeerConnection();
}

const std::string& PeerSession::participantId() const {
    return m_participantId;
}

const std::string& PeerSession::roomId() const {
    return m_roomId;
}

const std::string& PeerSession::signalingId() const {
    return m_signalingId;
}

void PeerSession::initPeerConnection(const std::shared_ptr<EventRouter>& router,
                                     rtc::Configuration config) {
    std::lock_guard lock(m_pcMutex);
    if (m_pc) {
        return;
    }
    m_eventRouter = router;
    try {
        m_pc = std::make_shared<rtc::PeerConnection>(std::move(config));
        auto weakSelf = weak_from_this();
        m_pc->onLocalDescription([weakSelf](rtc::Description description) {
            if (auto self = weakSelf.lock()) {
                self->onLocalDescription(std::move(description));
            }
        });
        m_pc->onLocalCandidate([weakSelf](rtc::Candidate candidate) {
            if (auto self = weakSelf.lock()) {
                self->onLocalCandidate(std::move(candidate));
            }
        });
        m_pc->onStateChange([weakSelf](rtc::PeerConnection::State state) {
            if (auto self = weakSelf.lock()) {
                self->onStateChange(state);
            }
        });
        m_pc->onIceStateChange([weakSelf](rtc::PeerConnection::IceState state) {
            if (auto self = weakSelf.lock()) {
                self->onIceStateChange(state);
            }
        });
    } catch (...) {
        m_pc.reset();
        m_eventRouter.reset();
        throw;
    }
}

void PeerSession::shutdownPeerConnection() {
    std::shared_ptr<rtc::PeerConnection> pc;
    {
        std::lock_guard lock(m_pcMutex);
        m_answerPromise.reset();
        pc = std::move(m_pc);
    }
    if (pc) {
        pc->resetCallbacks();
        pc->close();
    }
    m_peerConnectedEmitted.store(false);
}

bool PeerSession::hasPeerConnection() const {
    std::lock_guard lock(m_pcMutex);
    return m_pc != nullptr;
}

std::optional<std::string> PeerSession::applyRemoteOffer(const std::string& sdp) {
    std::shared_ptr<rtc::PeerConnection> pc;
    std::shared_ptr<std::promise<std::string>> prom;
    {
        std::lock_guard lock(m_pcMutex);
        if (!m_pc) {
            return std::nullopt;
        }
        pc = m_pc;
        prom = std::make_shared<std::promise<std::string>>();
        m_answerPromise = prom;
    }

    try {
        pc->setRemoteDescription(rtc::Description(sdp, rtc::Description::Type::Offer));
    } catch (const std::exception&) {
        std::lock_guard lock(m_pcMutex);
        m_answerPromise.reset();
        return std::nullopt;
    }

    std::future<std::string> fut = prom->get_future();
    constexpr auto answerWait = std::chrono::seconds(15);
    if (fut.wait_for(answerWait) != std::future_status::ready) {
        std::lock_guard lock(m_pcMutex);
        m_answerPromise.reset();
        return std::nullopt;
    }

    try {
        return fut.get();
    } catch (...) {
        return std::nullopt;
    }
}

bool PeerSession::addRemoteIceCandidate(const std::string& candidate, const std::string& sdpMid) {
    std::shared_ptr<rtc::PeerConnection> pc;
    {
        std::lock_guard lock(m_pcMutex);
        if (!m_pc) {
            return false;
        }
        pc = m_pc;
    }
    try {
        pc->addRemoteCandidate(rtc::Candidate(candidate, sdpMid));
        return true;
    } catch (const std::exception&) {
        return false;
    }
}

void PeerSession::onLocalDescription(rtc::Description description) {
    std::shared_ptr<std::promise<std::string>> prom;
    {
        std::lock_guard lock(m_pcMutex);
        prom = std::move(m_answerPromise);
    }
    if (!prom) {
        return;
    }
    if (description.type() == rtc::Description::Type::Answer) {
        try {
            prom->set_value(std::string(description));
        } catch (...) {
            // already satisfied or destroyed
        }
    } else {
        try {
            prom->set_exception(std::make_exception_ptr(
                std::runtime_error("expected answer from local description")));
        } catch (...) {
        }
    }
}

void PeerSession::onLocalCandidate(rtc::Candidate candidate) {
    emitIceCandidate(candidate);
}

void PeerSession::onStateChange(rtc::PeerConnection::State state) {
    using S = rtc::PeerConnection::State;
    if (state == S::Connected) {
        emitPeerConnected();
    } else if (state == S::Failed) {
        emitPeerDisconnected("failed");
        m_peerConnectedEmitted.store(false);
    } else if (state == S::Closed) {
        emitPeerDisconnected("closed");
        m_peerConnectedEmitted.store(false);
    }
}

void PeerSession::onIceStateChange(rtc::PeerConnection::IceState state) {
    using I = rtc::PeerConnection::IceState;
    if (state == I::Connected || state == I::Completed) {
        emitPeerConnected();
    } else if (state == I::Failed) {
        emitPeerDisconnected("ice_failed");
    }
}

void PeerSession::emitIceCandidate(const rtc::Candidate& candidate) {
    auto router = m_eventRouter;
    if (!router) {
        return;
    }
    sfu::SFUEvent ev;
    ev.set_room_id(m_roomId);
    ev.set_participant_id(m_participantId);
    auto* ice = ev.mutable_ice_candidate();
    ice->set_candidate(std::string(candidate));
    ice->set_sdp_mid(candidate.mid());
    router->routeEvent(m_signalingId, ev);
}

void PeerSession::emitPeerConnected() {
    if (m_peerConnectedEmitted.exchange(true)) {
        return;
    }
    auto router = m_eventRouter;
    if (!router) {
        return;
    }
    sfu::SFUEvent ev;
    ev.set_room_id(m_roomId);
    ev.set_participant_id(m_participantId);
    auto* connected = ev.mutable_peer_connected();
    connected->set_transport_type("udp");
    router->routeEvent(m_signalingId, ev);
}

void PeerSession::emitPeerDisconnected(const std::string& reason) {
    auto router = m_eventRouter;
    if (!router) {
        return;
    }
    sfu::SFUEvent ev;
    ev.set_room_id(m_roomId);
    ev.set_participant_id(m_participantId);
    auto* disc = ev.mutable_peer_disconnected();
    disc->set_reason(reason);
    router->routeEvent(m_signalingId, ev);
}
