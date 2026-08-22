(ns jepsen.openraft.client
  (:require [cheshire.core :as json]
            [cheshire.factory :as json-factory]
            [cheshire.parse :as json-parse]
            [clojure.string :as str]
            [jepsen.openraft.interruption :as interruption])
  (:import (com.fasterxml.jackson.core JsonFactory
                                       JsonParseException
                                       JsonParser$Feature
                                       JsonToken)
           (java.net ConnectException URI)
           (java.net.http HttpClient
                          HttpConnectTimeoutException
                          HttpRequest
                          HttpRequest$BodyPublishers
                          HttpResponse$BodyHandlers
                          HttpTimeoutException)
           (java.time Duration)))

(def default-api-port 21001)
(def default-raft-port 22001)

(def ^:private max-u64 18446744073709551615N)
(def ^:private response-body-preview-limit 1024)
(def ^:private modeled-error-variants
  #{:ForwardToLeader :QuorumNotEnough})

(def ^:private ^JsonFactory preflight-json-factory
  (json-factory/make-json-factory {}))

(def ^:private ^JsonFactory strict-json-factory
  (doto (json-factory/make-json-factory {})
    (.enable JsonParser$Feature/STRICT_DUPLICATE_DETECTION)))

(def ^:private http-client
  (-> (HttpClient/newBuilder)
      (.connectTimeout (Duration/ofSeconds 2))
      (.build)))

(defn node-host [node]
  (if (keyword? node)
    (name node)
    (str node)))

(defn api-endpoint [test node]
  (str (node-host node) ":" (:api-port test default-api-port)))

(defn raft-addr [test node]
  (str (node-host node) ":" (:raft-port test default-raft-port)))

(defn- node-url [node path]
  (let [base (if (re-find #"^https?://" node)
               node
               (str "http://" node))]
    (str (str/replace base #"/+$" "")
         (if (str/starts-with? path "/")
           path
           (str "/" path)))))

(defn- request-builder [endpoint path]
  (doto (HttpRequest/newBuilder (URI/create (node-url endpoint path)))
    (.timeout (Duration/ofSeconds 5))))

(defn- response-evidence [response]
  (let [body (:body response)]
    (cond-> (select-keys response [:method :uri :status])
      (string? body)
      (assoc :body-character-count (count body)
             :body-preview (subs body
                                 0
                                 (min response-body-preview-limit
                                      (count body)))))))

(defn- send! [request]
  (let [request-info {:method (.method request)
                      :uri (str (.uri request))}
        response (try
                   (.send http-client request (HttpResponse$BodyHandlers/ofString))
                   (catch HttpConnectTimeoutException e
                     (throw (ex-info "HTTP connection timed out"
                                     (assoc request-info :kind :unreachable)
                                     e)))
                   (catch ConnectException e
                     (throw (ex-info "HTTP connection failed"
                                     (assoc request-info :kind :unreachable)
                                     e)))
                   (catch HttpTimeoutException e
                     (throw (ex-info "HTTP request timed out"
                                     (assoc request-info :kind :request-timeout)
                                     e)))
                   (catch java.io.IOException e
                     (when (interruption/interruption? e)
                       (.interrupt (Thread/currentThread))
                       (throw e))
                     (throw (ex-info "HTTP request failed"
                                     (assoc request-info :kind :transport-error)
                                     e)))
                   (catch InterruptedException e
                     (.interrupt (Thread/currentThread))
                     (throw e)))
        status (.statusCode response)
        body (.body response)
        result (assoc request-info
                      :status status
                      :body body)]
    (when-not (= 200 status)
      (throw (ex-info (str "HTTP request failed with status " status)
                      (assoc (response-evidence result) :kind :http-error))))
    result))

(defn- invalid-response! [response reason details]
  (throw (ex-info "OpenRaft API returned an invalid response"
                  (merge {:kind :invalid-response
                          :reason reason
                          :response (response-evidence response)}
                         details))))

(defn- parse-body [response]
  (try
    (with-open [parser (.createParser preflight-json-factory
                                      ^String (:body response))]
      (let [token (.nextToken parser)]
        (when (nil? token)
          (throw (JsonParseException.
                  parser
                  "OpenRaft API response has no JSON document")))
        (when-not (= JsonToken/START_OBJECT token)
          (.skipChildren parser)
          (when (.nextToken parser)
            (throw (JsonParseException.
                    parser
                    "Trailing content after OpenRaft API response")))
          (invalid-response! response
                             :response-union-not-map
                             {:response-arm-count nil}))))
    (with-open [parser (.createParser strict-json-factory
                                      ^String (:body response))]
      (let [body (json-parse/parse-strict parser
                                          true
                                          ::missing-json-root
                                          nil)]
        (when (= ::missing-json-root body)
          (throw (JsonParseException.
                  parser
                  "OpenRaft API response has no JSON document")))
        (when (.nextToken parser)
          (throw (JsonParseException.
                  parser
                  "Trailing content after OpenRaft API response")))
        body))
    (catch JsonParseException e
      (throw (ex-info "Failed to parse OpenRaft API response"
                      {:kind :invalid-json
                       :response (response-evidence response)}
                      e)))))

(defn- invalid-union-reason [body]
  (cond
    (empty? body) :missing-response-arm
    (and (contains? body :Ok)
         (contains? body :Err)) :ambiguous-response-arms
    (not-any? #{:Ok :Err} (keys body)) :unknown-response-arm
    :else :invalid-response-union))

(defn- ok-value [response]
  (let [body (parse-body response)
        arms (set (filter #{:Ok :Err} (keys body)))]
    (cond
      (= #{:Ok} arms)
      (:Ok body)

      (= #{:Err} arms)
      (let [error (:Err body)]
        (if (and (map? error) (= 1 (count error)))
          (let [[variant payload] (first error)]
            (if (and (modeled-error-variants variant)
                     (not (map? payload)))
              (invalid-response! response
                                 :invalid-error-payload
                                 {:error-variant variant})
              (throw (ex-info "OpenRaft API returned Err"
                              {:kind :openraft-error
                               :error error
                               :response (response-evidence response)}))))
          (invalid-response! response
                             :invalid-error-union
                             {:error-arm-count (when (map? error)
                                                 (count error))})))

      :else
      (invalid-response! response
                         (invalid-union-reason body)
                         {:response-arm-count (count arms)}))))

(defn- versioned-value? [value]
  (and (map? value)
       (contains? value :value)
       (string? (:value value))
       (contains? value :version)
       (integer? (:version value))
       (<= 0 (:version value) max-u64)))

(defn- response-value [response payload path allow-nil?]
  (let [value (get-in payload path ::missing)]
    (cond
      (= ::missing value)
      (invalid-response! response
                         :missing-payload-field
                         {:payload-path path})

      (nil? value)
      (if allow-nil?
        nil
        (invalid-response! response
                           :nil-payload-field
                           {:payload-path path}))

      (versioned-value? value)
      value

      :else
      (invalid-response! response
                         :invalid-versioned-value
                         {:payload-path path}))))

(defn- require-echoed-value [response versioned-value expected]
  (when (and versioned-value
             (not= expected (:value versioned-value)))
    (invalid-response! response
                       :unexpected-payload-value
                       {:payload-path [:data :value :value]}))
  versioned-value)

(defn- require-newer-cas-version [response versioned-value expected-version]
  (when (and versioned-value
             (<= (:version versioned-value) expected-version))
    (invalid-response! response
                       :non-increasing-cas-version
                       {:payload-path [:data :value :version]}))
  versioned-value)

(defn- forward-to-leader? [e]
  ;; contains? returns false for a nil map, so no nil guard is needed.
  (contains? (:error (ex-data e)) :ForwardToLeader))

(defn- quorum-not-enough? [e]
  (contains? (:error (ex-data e)) :QuorumNotEnough))

(defn- forward-endpoint [e]
  (get-in (ex-data e)
          [:error :ForwardToLeader :leader_node :data]))

(defn- unreachable? [e]
  (= :unreachable (:kind (ex-data e))))

(defn retryable-read-error?
  "Returns true when a read can safely try another node after this error."
  [e]
  (let [kind (:kind (ex-data e))]
    (or (#{:request-timeout :transport-error} kind)
        (quorum-not-enough? e))))

(defn- reroute-next-operation? [e]
  (retryable-read-error? e))

(defn- next-endpoint [endpoints attempted e]
  ;; A ForwardToLeader with an empty data field carries no usable address, and
  ;; "" is truthy in Clojure, so use seq to fall back to the known endpoints.
  (let [forward (forward-endpoint e)
        candidates (if (seq forward)
                     (cons forward endpoints)
                     endpoints)]
    (first (remove attempted candidates))))

(defn with-leader!
  "Runs request against the cached leader and follows safe routing failures.

  ForwardToLeader and connection-establishment failures prove that the request
  was not applied, so retrying them does not duplicate a mutation. The optional
  retryable? predicate allows side-effect-free operations to retry additional
  failures. An ambiguous failure is returned to the workload, but the next
  operation starts from another endpoint."
  ([leader-endpoint endpoints request]
   (with-leader! leader-endpoint endpoints request (constantly false)))
  ([leader-endpoint endpoints request retryable?]
   (loop [endpoint @leader-endpoint
          attempted #{}]
     (let [attempted (conj attempted endpoint)
           [result value] (try
                            [:ok (request endpoint)]
                            (catch Exception e
                              [:error e]))]
       (if (= :ok result)
         (do
           (reset! leader-endpoint endpoint)
           value)
         (if (or (forward-to-leader? value)
                 (unreachable? value)
                 (retryable? value))
           (if-let [endpoint (next-endpoint endpoints attempted value)]
             (recur endpoint attempted)
             (throw value))
           (do
             (when (reroute-next-operation? value)
               (when-let [endpoint (next-endpoint endpoints attempted value)]
                 (reset! leader-endpoint endpoint)))
             (throw value))))))))

(defn get! [endpoint path]
  (-> (request-builder endpoint path)
      (.GET)
      (.build)
      send!))

(defn post! [endpoint path body]
  (-> (request-builder endpoint path)
      (.header "Content-Type" "application/json")
      (.POST (HttpRequest$BodyPublishers/ofString (json/generate-string body)))
      (.build)
      send!))

(defn metrics! [endpoint]
  (ok-value (get! endpoint "/metrics")))

(defn init! [endpoint]
  (ok-value (post! endpoint "/init" [])))

(defn add-learner! [endpoint node-id api-addr raft-addr]
  (ok-value (post! endpoint "/add-learner"
                   {:node_id node-id
                    :api_addr api-addr
                    :raft_addr raft-addr})))

(defn change-membership! [endpoint node-ids]
  (ok-value (post! endpoint "/change-membership" node-ids)))

;; These functions make one HTTP attempt. KVClient performs leader routing and
;; classifies errors based on whether the operation may have taken effect.
(defn write! [endpoint key value]
  (let [response (post! endpoint "/write"
                        {:Set {:key key
                               :value value}})
        payload (ok-value response)
        versioned-value (response-value response
                                        payload
                                        [:data :value]
                                        false)]
    (require-echoed-value response versioned-value value)))

(defn cas! [endpoint key expected-version value]
  (let [response (post! endpoint "/write"
                        {:CompareAndSet {:key key
                                         :expected_version expected-version
                                         :value value}})
        payload (ok-value response)
        versioned-value (response-value response
                                        payload
                                        [:data :value]
                                        true)
        versioned-value (require-echoed-value response
                                              versioned-value
                                              value)]
    (require-newer-cas-version response
                               versioned-value
                               expected-version)))

(defn linearizable-read! [endpoint key]
  (let [response (post! endpoint "/linearizable_read" key)
        payload (ok-value response)]
    (response-value response payload [:value] true)))
