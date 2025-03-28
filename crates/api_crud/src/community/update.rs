use super::check_community_visibility_allowed;
use activitypub_federation::config::Data;
use actix_web::web::Json;
use chrono::Utc;
use lemmy_api_common::{
  build_response::build_community_response,
  community::{CommunityResponse, EditCommunity},
  context::LemmyContext,
  send_activity::{ActivityChannel, SendActivityData},
  utils::{
    check_community_mod_action,
    get_url_blocklist,
    local_site_to_slur_regex,
    process_markdown_opt,
  },
};
use lemmy_db_schema::{
  source::{
    actor_language::{CommunityLanguage, SiteLanguage},
    community::{Community, CommunityUpdateForm},
    local_site::LocalSite,
  },
  traits::Crud,
  utils::diesel_string_update,
};
use lemmy_db_views::structs::LocalUserView;
use lemmy_utils::{
  error::{LemmyErrorExt, LemmyErrorType, LemmyResult},
  utils::{slurs::check_slurs_opt, validation::is_valid_body_field},
};

pub async fn update_community(
  data: Json<EditCommunity>,
  context: Data<LemmyContext>,
  local_user_view: LocalUserView,
) -> LemmyResult<Json<CommunityResponse>> {
  let local_site = LocalSite::read(&mut context.pool()).await?;

  let slur_regex = local_site_to_slur_regex(&local_site);
  let url_blocklist = get_url_blocklist(&context).await?;
  check_slurs_opt(&data.title, &slur_regex)?;

  let sidebar = diesel_string_update(
    process_markdown_opt(&data.sidebar, &slur_regex, &url_blocklist, &context)
      .await?
      .as_deref(),
  );

  if let Some(Some(sidebar)) = &sidebar {
    is_valid_body_field(sidebar, false)?;
  }

  check_community_visibility_allowed(data.visibility, &local_user_view)?;
  let description = diesel_string_update(data.description.as_deref());

  let old_community = Community::read(&mut context.pool(), data.community_id).await?;

  // Verify its a mod (only mods can edit it)
  check_community_mod_action(
    &local_user_view.person,
    &old_community,
    false,
    &mut context.pool(),
  )
  .await?;

  let community_id = data.community_id;
  if let Some(languages) = data.discussion_languages.clone() {
    let site_languages = SiteLanguage::read_local_raw(&mut context.pool()).await?;
    // check that community languages are a subset of site languages
    // https://stackoverflow.com/a/64227550
    let is_subset = languages.iter().all(|item| site_languages.contains(item));
    if !is_subset {
      Err(LemmyErrorType::LanguageNotAllowed)?
    }
    CommunityLanguage::update(&mut context.pool(), languages, community_id).await?;
  }

  let community_form = CommunityUpdateForm {
    title: data.title.clone(),
    sidebar,
    description,
    nsfw: data.nsfw,
    posting_restricted_to_mods: data.posting_restricted_to_mods,
    visibility: data.visibility,
    updated: Some(Some(Utc::now())),
    ..Default::default()
  };

  let community_id = data.community_id;
  let community = Community::update(&mut context.pool(), community_id, &community_form)
    .await
    .with_lemmy_type(LemmyErrorType::CouldntUpdateCommunity)?;

  ActivityChannel::submit_activity(
    SendActivityData::UpdateCommunity(local_user_view.person.clone(), community),
    &context,
  )?;

  build_community_response(&context, local_user_view, community_id).await
}
