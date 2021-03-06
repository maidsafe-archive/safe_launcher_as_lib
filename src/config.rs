// Copyright 2015 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under (1) the MaidSafe.net Commercial License,
// version 1.0 or later, or (2) The General Public License (GPL), version 3, depending on which
// licence you accepted on initial access to the Software (the "Licences").
//
// By contributing code to the SAFE Network Software, or to this project generally, you agree to be
// bound by the terms of the MaidSafe Contributor Agreement, version 1.0.  This, along with the
// Licenses can be found in the root directory of this project at LICENSE, COPYING and CONTRIBUTOR.
//
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied.
//
// Please review the Licences for the specific language governing permissions and limitations
// relating to use of the SAFE Network Software.

pub const SAFE_DRIVE_DIR_NAME: &'static str = "SAFEDrive";
pub const LAUNCHER_NONCE_LENGTH: usize = 13;
pub const LAUNCHER_GLOBAL_DIRECTORY_NAME: &'static str = "LauncherReservedDirectory";
pub const LAUNCHER_LOCAL_CONFIG_FILE_NAME: &'static str = ".launcher.config";
pub const LAUNCHER_GLOBAL_CONFIG_FILE_NAME: &'static str = "LauncherSpecificConfigurationFile";
pub const MAX_ALLOWED_READ_PAYLOAD_SIZE_BYTES: u64 = 300 * 1024 * 1024;

use rustc_serialize::base64::{CharacterSet, Config, Newline};

pub fn get_base64_config() -> Config {
    Config {
        char_set: CharacterSet::Standard,
        newline: Newline::LF,
        pad: true,
        line_length: None,
    }
}
