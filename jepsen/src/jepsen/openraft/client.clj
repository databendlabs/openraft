(ns jepsen.openraft.client
  (:require [cheshire.core :as json]
            [clojure.string :as str])
  (:import (java.net ConnectException URI)
           (java.net.http HttpClient
                          HttpConnectTimeoutException
                          HttpRequest
                          HttpRequest$BodyPublishers
                          HttpResponse$BodyHandlers
                          HttpTimeoutException)
           (java.time Duration)))

(def default-api-port 21001)
(def default-raft-port 22001)

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
                     (throw (ex-info "HTTP request failed"
                                     (assoc request-info :kind :transport-error)
                                     e)))
                   (catch InterruptedException e
                     (.interrupt (Thread/currentThread))
                     (throw (ex-info "HTTP request interrupted"
                                     (assoc request-info :kind :interrupted)
                                     e))))
        status (.statusCode response)
        body (.body response)
        result (assoc request-info
                      :status status
                      :body body)]
    (when-not (= 200 status)
      (throw (ex-info (str "HTTP request failed with status " status)
                      (assoc result :kind :http-error))))
    result))

(defn- parse-body [response]
  (try
    (json/parse-string (:body response) true)
    (catch Exception e
      (throw (ex-info "Failed to parse OpenRaft API response"
                      {:kind :invalid-json
                       :response response}
                      e)))))

(defn- ok-value [response]
  (let [body (parse-body response)]
    (cond
      (contains? body :Ok) (:Ok body)
      (contains? body :Err) (throw (ex-info "OpenRaft API returned Err"
                                            {:kind :openraft-error
                                             :error (:Err body)
                                             :response response}))
      :else body)))

(defn- forward-to-leader? [e]
  (contains? (or (:error (ex-data e)) {}) :ForwardToLeader))

(defn- quorum-not-enough? [e]
  (contains? (or (:error (ex-data e)) {}) :QuorumNotEnough))

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
  (let [forward (forward-endpoint e)
        candidates (if forward
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
  (-> (post! endpoint "/write"
             {:Set {:key key
                    :value value}})
      ok-value
      :data
      :value))

(defn cas! [endpoint key expected-version value]
  (-> (post! endpoint "/write"
             {:CompareAndSet {:key key
                              :expected_version expected-version
                              :value value}})
      ok-value
      :data
      :value))

(defn linearizable-read! [endpoint key]
  (-> (post! endpoint "/linearizable_read" key)
      ok-value
      :value))
