#include "sfu_service_impl.h"

#include "event_router.h"
#include "sfu_runtime.h"

#include "signaling.pb.h"

#include <thread>

#include "absl/log/log.h"

SfuServiceImpl::SfuServiceImpl(std::shared_ptr<SfuRuntime> runtime)
    : m_runtime(std::move(runtime)) {}

grpc::Status SfuServiceImpl::CreatePeer(grpc::ServerContext* context,
                                        const sfu::CreatePeerRequest* request,
                                        sfu::CreatePeerResponse* response) {
    (void)context;
    ABSL_LOG(INFO) << "CreatePeer room_id=" << request->room_id()
                   << " participant_id=" << request->participant_id()
                   << " signaling_id=" << request->signaling_id();
    std::string err;
    if (!m_runtime->addPeer(request->room_id(), request->participant_id(), request->signaling_id(),
                            err)) {
        response->set_success(false);
        response->set_error_message(err);
        return grpc::Status::OK;
    }
    response->set_success(true);
    return grpc::Status::OK;
}

grpc::Status SfuServiceImpl::DeletePeer(grpc::ServerContext* context,
                                        const sfu::DeletePeerRequest* request,
                                        sfu::DeletePeerResponse* response) {
    (void)context;
    ABSL_LOG(INFO) << "DeletePeer room_id=" << request->room_id()
                   << " participant_id=" << request->participant_id()
                   << " reason=" << request->reason();
    if (!m_runtime->removePeer(request->room_id(), request->participant_id())) {
        ABSL_LOG(WARNING) << "DeletePeer unknown peer room_id=" << request->room_id()
                          << " participant_id=" << request->participant_id();
    }
    response->set_success(true);
    return grpc::Status::OK;
}

grpc::Status SfuServiceImpl::HandleSDP(grpc::ServerContext* context,
                                       const sfu::HandleSDPRequest* request,
                                       sfu::HandleSDPResponse* response) {
    (void)context;
    const auto sdpType = static_cast<sfu::SdpType>(request->type());
    ABSL_LOG(INFO) << "HandleSDP room_id=" << request->room_id()
                   << " participant_id=" << request->participant_id()
                   << " type=" << static_cast<int>(sdpType)
                   << " sdp_bytes=" << request->sdp().size();

    auto session =
        m_runtime->findPeerSession(request->room_id(), request->participant_id());
    if (!session || !session->hasPeerConnection()) {
        response->set_success(false);
        response->set_error_message("unknown peer or PeerConnection not initialized");
        response->set_type(sfu::SdpType::SDP_TYPE_UNSPECIFIED);
        return grpc::Status::OK;
    }

    if (sdpType == sfu::SdpType::SDP_TYPE_ANSWER) {
        response->set_success(false);
        response->set_error_message(
            "client SDP answer not supported yet; send an offer after CreatePeer");
        response->set_type(sfu::SdpType::SDP_TYPE_UNSPECIFIED);
        return grpc::Status::OK;
    }

    if (sdpType != sfu::SdpType::SDP_TYPE_OFFER && sdpType != sfu::SdpType::SDP_TYPE_UNSPECIFIED) {
        response->set_success(false);
        response->set_error_message("unsupported SDP type");
        response->set_type(sfu::SdpType::SDP_TYPE_UNSPECIFIED);
        return grpc::Status::OK;
    }

    std::optional<std::string> answer = session->applyRemoteOffer(request->sdp());
    if (!answer) {
        response->set_success(false);
        response->set_error_message("failed to apply offer or timed out waiting for answer");
        response->set_type(sfu::SdpType::SDP_TYPE_UNSPECIFIED);
        return grpc::Status::OK;
    }

    response->set_success(true);
    response->set_sdp(std::move(*answer));
    response->set_type(sfu::SdpType::SDP_TYPE_ANSWER);
    return grpc::Status::OK;
}

grpc::Status SfuServiceImpl::AddICECandidate(grpc::ServerContext* context,
                                             const sfu::AddICECandidateRequest* request,
                                             sfu::AddICECandidateResponse* response) {
    (void)context;
    ABSL_LOG(INFO) << "AddICECandidate room_id=" << request->room_id()
                   << " participant_id=" << request->participant_id();

    auto session =
        m_runtime->findPeerSession(request->room_id(), request->participant_id());
    if (!session || !session->addRemoteIceCandidate(request->candidate(), request->sdp_mid())) {
        response->set_success(false);
        response->set_error_message("unknown peer or failed to add ICE candidate");
        return grpc::Status::OK;
    }
    response->set_success(true);
    return grpc::Status::OK;
}

grpc::Status SfuServiceImpl::SubscribeEvents(grpc::ServerContext* context,
                                             const sfu::SubscribeEventsRequest* request,
                                             grpc::ServerWriter<sfu::SFUEvent>* writer) {
    const std::string& signalingId = request->signaling_id();
    ABSL_LOG(INFO) << "SubscribeEvents signaling_id=" << signalingId;

    auto router = m_runtime->eventRouter();
    auto sub = std::make_shared<EventRouter::Subscriber>();
    sub->m_writer = writer;
    router->registerSignaling(signalingId, sub);

    while (!context->IsCancelled()) {
        sfu::SFUEvent ev = m_runtime->buildHeartbeatEvent();
        {
            std::lock_guard lock(sub->m_writeMutex);
            if (!writer->Write(ev)) {
                break;
            }
        }
        std::this_thread::sleep_for(m_heartbeatInterval);
    }

    router->unregisterSignaling(signalingId, sub);
    return grpc::Status::OK;
}

grpc::Status SfuServiceImpl::Ping(grpc::ServerContext* context,
                                  const sfu::PingRequest* request,
                                  sfu::PingResponse* response) {
    (void)context;
    (void)request;
    response->set_ok(true);
    return grpc::Status::OK;
}
