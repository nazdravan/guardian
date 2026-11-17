//! Admin RPCs implemented by later tasks.

use guardian_proto::v1 as pb;
use guardian_proto::v1::{
    ComputerIdRequest, ComputerList, ConsoleList, CreateEnrollCodeRequest, CreateUserRequest,
    Empty, EnrollCode, IdRequest, SetUserRoomsRequest, UpdateComputerRequest, UpdateUserRequest,
    User, UserList,
};
use tonic::{Request, Response, Status};

use super::admin::AdminSvc;

impl AdminSvc {
    pub(crate) async fn list_users_impl(
        &self,
        _r: Request<Empty>,
    ) -> Result<Response<UserList>, Status> {
        Err(Status::unimplemented("ListUsers"))
    }
    pub(crate) async fn create_user_impl(
        &self,
        _r: Request<CreateUserRequest>,
    ) -> Result<Response<User>, Status> {
        Err(Status::unimplemented("CreateUser"))
    }
    pub(crate) async fn update_user_impl(
        &self,
        _r: Request<UpdateUserRequest>,
    ) -> Result<Response<User>, Status> {
        Err(Status::unimplemented("UpdateUser"))
    }
    pub(crate) async fn set_user_rooms_impl(
        &self,
        _r: Request<SetUserRoomsRequest>,
    ) -> Result<Response<User>, Status> {
        Err(Status::unimplemented("SetUserRooms"))
    }
    pub(crate) async fn list_computers_impl(
        &self,
        _r: Request<Empty>,
    ) -> Result<Response<ComputerList>, Status> {
        Err(Status::unimplemented("ListComputers"))
    }
    pub(crate) async fn update_computer_impl(
        &self,
        _r: Request<UpdateComputerRequest>,
    ) -> Result<Response<pb::Computer>, Status> {
        Err(Status::unimplemented("UpdateComputer"))
    }
    pub(crate) async fn revoke_computer_impl(
        &self,
        _r: Request<ComputerIdRequest>,
    ) -> Result<Response<Empty>, Status> {
        Err(Status::unimplemented("RevokeComputer"))
    }
    pub(crate) async fn create_enroll_code_impl(
        &self,
        _r: Request<CreateEnrollCodeRequest>,
    ) -> Result<Response<EnrollCode>, Status> {
        Err(Status::unimplemented("CreateEnrollCode"))
    }
    pub(crate) async fn list_consoles_impl(
        &self,
        _r: Request<Empty>,
    ) -> Result<Response<ConsoleList>, Status> {
        Err(Status::unimplemented("ListConsoles"))
    }
    pub(crate) async fn revoke_console_impl(
        &self,
        _r: Request<IdRequest>,
    ) -> Result<Response<Empty>, Status> {
        Err(Status::unimplemented("RevokeConsole"))
    }
}
