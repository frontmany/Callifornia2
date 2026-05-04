#include "peer_session.h"

#include "event_router.h"

#include "signaling.pb.h"

#include <chrono>
#include <future>
#include <regex>
#include <stdexcept>
#include <utility>

#include "absl/log/log.h"

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
        m_pc->onTrack([weakSelf](std::shared_ptr<rtc::Track> track) {
            if (auto self = weakSelf.lock()) {
                self->onTrack(track);
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

std::optional<std::string> PeerSession::applyRemoteOffer(const std::string& sdp,
                                                         std::string& errorMessage) {
    if (m_negotiationInProgress.exchange(true)) {
        errorMessage = "negotiation already in progress for peer";
        return std::nullopt;
    }
    struct NegotiationGuard {
        std::atomic<bool>& flag;
        ~NegotiationGuard() { flag.store(false); }
    } guard{m_negotiationInProgress};

    if (!validateFixedSlotsOffer(sdp, errorMessage)) {
        return std::nullopt;
    }

    std::shared_ptr<rtc::PeerConnection> pc;
    std::shared_ptr<std::promise<std::string>> prom;
    {
        std::lock_guard lock(m_pcMutex);
        if (!m_pc) {
            errorMessage = "PeerConnection is not initialized";
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
        errorMessage = "failed to apply remote SDP offer";
        return std::nullopt;
    }

    std::future<std::string> fut = prom->get_future();
    constexpr auto answerWait = std::chrono::seconds(15);
    if (fut.wait_for(answerWait) != std::future_status::ready) {
        std::lock_guard lock(m_pcMutex);
        m_answerPromise.reset();
        errorMessage = "timed out waiting for local SDP answer";
        return std::nullopt;
    }

    try {
        return fut.get();
    } catch (...) {
        errorMessage = "failed to read local SDP answer";
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

void PeerSession::setTrackHandlers(TrackAddedHandler added, TrackRemovedHandler removed) {
    std::lock_guard lock(m_tracksMutex);
    m_trackAddedHandler = std::move(added);
    m_trackRemovedHandler = std::move(removed);
}

std::shared_ptr<rtc::Track> PeerSession::addRelayTrackIfMissing(
    const std::string& relayTrackId,
    const rtc::Description::Media& sourceDescription,
    std::string& errorMessage) {
    {
        std::lock_guard lock(m_tracksMutex);
        auto it = m_relayTracks.find(relayTrackId);
        if (it != m_relayTracks.end()) {
            return it->second;
        }
    }

    std::shared_ptr<rtc::PeerConnection> pc;
    {
        std::lock_guard lock(m_pcMutex);
        if (!m_pc) {
            errorMessage = "PeerConnection is not initialized";
            return nullptr;
        }
        pc = m_pc;
    }

    rtc::Description::Media relayDescription = sourceDescription;
    relayDescription.setDirection(rtc::Description::Direction::SendOnly);
    std::shared_ptr<rtc::Track> relayTrack;
    try {
        relayTrack = pc->addTrack(relayDescription);
    } catch (const std::exception& ex) {
        errorMessage = ex.what();
        return nullptr;
    }
    {
        std::lock_guard lock(m_tracksMutex);
        auto [it, _] = m_relayTracks.emplace(relayTrackId, relayTrack);
        return it->second;
    }
}

void PeerSession::removeRelayTrack(const std::string& relayTrackId) {
    std::shared_ptr<rtc::Track> relay;
    {
        std::lock_guard lock(m_tracksMutex);
        auto it = m_relayTracks.find(relayTrackId);
        if (it == m_relayTracks.end()) {
            return;
        }
        relay = std::move(it->second);
        m_relayTracks.erase(it);
    }
    if (!relay) {
        return;
    }
    try {
        relay->close();
    } catch (...) {
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

void PeerSession::onTrack(const std::shared_ptr<rtc::Track>& track) {
    if (!track) {
        return;
    }
    const auto desc = track->description();
    const std::string mid = desc.mid();
    const std::string kind = desc.type();
    const std::string sourceType = sourceTypeFromMid(mid);
    const std::string trackId = sourceType + ":" + mid;
    TrackAddedHandler addedHandler;
    TrackRemovedHandler removedHandler;
    {
        std::lock_guard lock(m_tracksMutex);
        m_trackKindsById[trackId] = kind;
        addedHandler = m_trackAddedHandler;
        removedHandler = m_trackRemovedHandler;
    }
    emitTrackAdded(trackId, kind, sourceType);
    if (addedHandler) {
        addedHandler(m_participantId, track, trackId, kind, sourceType);
    }
    track->onClosed([weakSelf = weak_from_this(), trackId]() {
        auto self = weakSelf.lock();
        if (!self) {
            return;
        }
        std::string kind = "unknown";
        TrackRemovedHandler removedHandler;
        {
            std::lock_guard lock(self->m_tracksMutex);
            auto it = self->m_trackKindsById.find(trackId);
            if (it != self->m_trackKindsById.end()) {
                kind = it->second;
                self->m_trackKindsById.erase(it);
            }
            removedHandler = self->m_trackRemovedHandler;
        }
        self->emitTrackRemoved(trackId, kind, "track_closed");
        if (removedHandler) {
            removedHandler(self->m_participantId, trackId, kind, "track_closed");
        }
    });
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
    routeEventOrLog(router, ev, "ice_candidate");
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
    routeEventOrLog(router, ev, "peer_connected");
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
    routeEventOrLog(router, ev, "peer_disconnected");
}

void PeerSession::emitTrackAdded(const std::string& trackId,
                                 const std::string& kind,
                                 const std::string& sourceType) {
    auto router = m_eventRouter;
    if (!router) {
        return;
    }
    sfu::SFUEvent ev;
    ev.set_room_id(m_roomId);
    ev.set_participant_id(m_participantId);
    auto* added = ev.mutable_track_added();
    added->set_track_id(trackId);
    added->set_kind(kind.empty() ? "unknown" : kind);
    added->set_codec(sourceType);
    added->set_is_sending(true);
    routeEventOrLog(router, ev, "track_added");
}

void PeerSession::emitTrackRemoved(const std::string& trackId,
                                   const std::string& kind,
                                   const std::string& reason) {
    auto router = m_eventRouter;
    if (!router) {
        return;
    }
    sfu::SFUEvent ev;
    ev.set_room_id(m_roomId);
    ev.set_participant_id(m_participantId);
    auto* removed = ev.mutable_track_removed();
    removed->set_track_id(trackId);
    removed->set_kind(kind.empty() ? "unknown" : kind);
    removed->set_reason(reason);
    routeEventOrLog(router, ev, "track_removed");
}

void PeerSession::routeEventOrLog(const std::shared_ptr<EventRouter>& router,
                                  const sfu::SFUEvent& event,
                                  const char* eventName) const {
    if (router->routeEvent(m_signalingId, event)) {
        return;
    }
    ABSL_LOG(WARNING) << "Failed to route SFU event "
                      << eventName
                      << " room_id="
                      << m_roomId
                      << " participant_id="
                      << m_participantId
                      << " signaling_id="
                      << m_signalingId;
}

bool PeerSession::validateFixedSlotsOffer(const std::string& sdp, std::string& errorMessage) {
    static const std::regex midRegex("^a=mid:(.+)$", std::regex_constants::icase);
    std::smatch match;
    std::size_t videoCount = 0;
    bool hasAudioSlot = false;
    bool hasCameraSlot = false;
    bool hasScreenSlot = false;

    std::string::size_type start = 0;
    while (start < sdp.size()) {
        const auto end = sdp.find('\n', start);
        std::string line = sdp.substr(start, end == std::string::npos ? std::string::npos : end - start);
        if (!line.empty() && line.back() == '\r') {
            line.pop_back();
        }
        if (std::regex_match(line, match, midRegex)) {
            const std::string mid = match[1].str();
            if (mid == "audio_main") {
                hasAudioSlot = true;
            } else if (mid == "video_camera") {
                hasCameraSlot = true;
                ++videoCount;
            } else if (mid == "video_screen") {
                hasScreenSlot = true;
                ++videoCount;
            }
        }
        if (end == std::string::npos) {
            break;
        }
        start = end + 1;
    }

    if (!hasAudioSlot) {
        errorMessage = "offer is missing required fixed slot a=mid:audio_main";
        return false;
    }
    if (!hasCameraSlot && !hasScreenSlot) {
        errorMessage = "offer must include at least one video fixed slot (video_camera or video_screen)";
        return false;
    }
    if (videoCount > 2) {
        errorMessage = "offer exceeds fixed slot policy: max two video mids";
        return false;
    }
    return true;
}

std::string PeerSession::sourceTypeFromMid(std::string mid) {
    if (mid == "video_camera") {
        return "camera";
    }
    if (mid == "video_screen") {
        return "screen";
    }
    if (mid == "audio_main") {
        return "audio";
    }
    return "unknown";
}
