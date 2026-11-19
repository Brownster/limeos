use super::*;

pub(super) async fn enable<B: Backend>(State(app): State<App<B>>, request: Request) -> Response {
    // Trusted process-local budget; JSON never supplies or renews this value.
    let deadline = Instant::now() + Duration::from_secs(10);
    let result = async {
        let (token, csrf) = operations::credentials(&app, request.headers())?;
        let principal = app
            .backend
            .session(token.clone(), Some(csrf.clone()))
            .await?
            .principal;
        let input: limeos_contracts::ObserveHealthEnable = operations::input(request).await?;
        if !limeos_contracts::password_well_formed(&input.password) {
            return Err(Error(ErrorCode::InvalidInput));
        }
        // Failed password attempts are budgeted per authenticated principal.
        password_attempt(&app, Attempt::Principal(&principal.id), |attempt| {
            app.backend
                .enable_observe_health(token, csrf, input, deadline, attempt)
        })
        .await
    }
    .await;
    match result {
        Ok(value) if value.reauthenticate => (
            [(
                "set-cookie",
                "__Host-limeos=; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=0",
            )],
            axum::Json(value),
        )
            .into_response(),
        Ok(value) => axum::Json(value).into_response(),
        Err(error) => failure(error),
    }
}

#[cfg(test)]
mod tests;
